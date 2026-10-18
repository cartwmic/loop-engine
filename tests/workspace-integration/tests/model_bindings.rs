//! Black-box proof of amendable per-role model bindings.
//! The Python scenario drives the built loop-engine and software-change programs on fresh
//! private catalogs; reviewer judgments are scripted and never claim semantic review.

#[path = "../../bounded_process.rs"]
mod bounded_process;

use std::process::Command;
use std::time::Duration;

const SCENARIOS: [&str; 7] = [
    "profile-identity",
    "unbound-amendments",
    "author-implementer-declarations",
    "cold-reader-return",
    "review-and-verdict-returns",
    "unmarked-amendment",
    "marked-amendment",
];

// bookends:LE-195 bookends:LE-196 bookends:LE-197 bookends:LE-198
// bookends:LE-199 bookends:LE-200
#[test]
fn model_binding_scenarios() {
    let repository = workspace_integration::repository_root();
    let temporary = tempfile::tempdir().expect("external fixture root");
    let output = temporary.path().join("model-bindings");
    let mut command = Command::new("python3");
    command
        .arg("-B")
        .arg(repository.join("tests/workspace-integration/tests/model_bindings/scenario.py"))
        .arg("--source-root")
        .arg(&repository)
        .arg("--output-root")
        .arg(&output)
        .current_dir(&repository);
    for (option, name) in [
        ("--engine", "loop-engine"),
        ("--provider", "software-change"),
    ] {
        command.arg(option).arg(workspace_integration::binary(name));
    }
    let result = bounded_process::run_with_deadline(
        &mut command,
        "model binding scenarios",
        Duration::from_secs(1500),
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let retained = temporary.keep();
            panic!("model binding scenario process failed: {error}; retained {retained:?}");
        }
    };
    if !result.status.success() {
        let retained = temporary.keep();
        panic!(
            "model binding scenarios failed; retained {retained:?}:\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let stdout = String::from_utf8(result.stdout).expect("scenario UTF-8");
    let summary: serde_json::Value =
        serde_json::from_str(stdout.lines().last().expect("scenario summary"))
            .expect("scenario summary JSON");
    let completed: Vec<&str> = summary["completed"]
        .as_array()
        .expect("completed scenarios")
        .iter()
        .map(|name| name.as_str().expect("scenario name"))
        .collect();
    assert_eq!(completed, SCENARIOS);
    assert_eq!(summary["failed"].as_array().unwrap().len(), 0);
    assert_eq!(summary["semantic_review"], false);
}
