//! Focused public recovery paths stay in the existing workspace target.
use std::process::Command;

#[test]
fn recovery_disposition_public_gate_and_history() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let work = std::env::temp_dir().join(format!(
        "recovery-disposition-central-{}",
        std::process::id()
    ));
    let output = Command::new("python3")
        .current_dir(&root)
        .args([
            "scripts/software-change-journey.py",
            "--mode",
            "source",
            "--engine",
        ])
        .arg(workspace_integration::binary("loop-engine"))
        .arg("--provider")
        .arg(workspace_integration::binary("software-change"))
        .args(["--data-root", ".", "--work-root"])
        .arg(&work)
        .args([
            "--profile",
            "crates/software-change-provider/data/configs/high-rigor.json",
            "--traversal-depth",
            "full",
            "--scenario",
            "dispositions",
        ])
        .output()
        .expect("run public dispositions journey");
    assert!(
        output.status.success(),
        "stdout={} stderr={} captures={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        work.display()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("recovery dispositions scenario passed")
    );
}

#[test]
fn recovery_steering_public_commission_and_delivery() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let work =
        std::env::temp_dir().join(format!("recovery-steering-central-{}", std::process::id()));
    let output = Command::new("python3")
        .current_dir(&root)
        .args([
            "scripts/software-change-journey.py",
            "--mode",
            "source",
            "--engine",
        ])
        .arg(workspace_integration::binary("loop-engine"))
        .arg("--provider")
        .arg(workspace_integration::binary("software-change"))
        .args(["--data-root", ".", "--work-root"])
        .arg(&work)
        .args([
            "--profile",
            "crates/software-change-provider/data/configs/high-rigor.json",
            "--traversal-depth",
            "full",
            "--scenario",
            "steering",
        ])
        .output()
        .expect("public steering journey");
    assert!(
        output.status.success(),
        "stdout={} stderr={} captures={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        work.display()
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("recovery steering passed"));
}

#[test]
fn recovery_batch_public_capture_candidates_and_confirmation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let work = std::env::temp_dir().join(format!("recovery-batch-central-{}", std::process::id()));
    let output = Command::new("python3")
        .current_dir(&root)
        .args([
            "scripts/software-change-journey.py",
            "--mode",
            "source",
            "--engine",
        ])
        .arg(workspace_integration::binary("loop-engine"))
        .arg("--provider")
        .arg(workspace_integration::binary("software-change"))
        .args(["--data-root", ".", "--work-root"])
        .arg(&work)
        .args([
            "--profile",
            "crates/software-change-provider/data/configs/high-rigor.json",
            "--traversal-depth",
            "full",
            "--scenario",
            "batched-review",
        ])
        .output()
        .expect("public batched-review journey");
    assert!(
        output.status.success(),
        "stdout={} stderr={} captures={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        work.display()
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("recovery batched-review scenario passed")
    );
}

#[test]
fn recovery_criterion_public_commands_checkpoint_repair_carry_and_terminal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let work =
        std::env::temp_dir().join(format!("recovery-criterion-central-{}", std::process::id()));
    let output = Command::new("python3")
        .current_dir(&root)
        .args([
            "scripts/software-change-journey.py",
            "--mode",
            "source",
            "--engine",
        ])
        .arg(workspace_integration::binary("loop-engine"))
        .arg("--provider")
        .arg(workspace_integration::binary("software-change"))
        .args(["--data-root", ".", "--work-root"])
        .arg(&work)
        .args([
            "--profile",
            "crates/software-change-provider/data/configs/high-rigor.json",
            "--traversal-depth",
            "full",
            "--scenario",
            "criteria",
        ])
        .output()
        .expect("public criterion journey");
    assert!(
        output.status.success(),
        "stdout={} stderr={} captures={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        work.display()
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("recovery criterion proof passed"));
}

#[test]
fn recovery_composed_public_terminal_and_outer_failure() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for broken in [false, true] {
        let work = std::env::temp_dir().join(format!(
            "recovery-composed-central-{}-{broken}",
            std::process::id()
        ));
        let output = Command::new("python3")
            .current_dir(&root)
            .arg(if broken {
                "tests/fixtures/break-composed-observation.py"
            } else {
                "scripts/software-change-journey.py"
            })
            .args(["--mode", "source", "--engine"])
            .arg(workspace_integration::binary("loop-engine"))
            .arg("--provider")
            .arg(workspace_integration::binary("software-change"))
            .args(["--data-root", ".", "--work-root"])
            .arg(&work)
            .args([
                "--profile",
                "crates/software-change-provider/data/configs/high-rigor.json",
                "--traversal-depth",
                "full",
                "--scenario",
                "composed-recovery",
            ])
            .output()
            .expect("public composed journey");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.success(),
            !broken,
            "stdout={stdout} stderr={stderr} captures={}",
            work.display()
        );
        if broken {
            assert!(stdout.contains("deliberately removed required terminal observation"));
            assert!(stderr.contains("AssertionError"));
            assert!(stdout.contains(
                "outer failure retained actual terminal, failed verdicts, history and raw captures"
            ));
            assert!(!stdout.contains("composed recovery journey passed"));
            // The failed observation cannot erase the real run's raw evidence.
            let fixture = std::fs::read_dir(&work)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| path.join("external.json").exists())
                .expect("retained composed external captures");
            assert!(fixture.join("validation-v1.json").exists());
            assert!(fixture.join("fresh-driver-criterion-failure.json").exists());
        } else {
            assert!(stdout.contains("composed recovery journey passed: normal completed; exceptional completed-with-overrides"));
        }
    }
}

#[test]
fn recovery_dispatch_unknown_scenario_fails_before_work() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .current_dir(&root)
        .args([
            "scripts/software-change-journey.py",
            "--mode",
            "source",
            "--engine",
            "/not-used/engine",
            "--provider",
            "/not-used/provider",
            "--data-root",
            ".",
            "--work-root",
            "/not-used/recovery-dispatch",
            "--profile",
            "/not-used/profile",
            "--traversal-depth",
            "full",
            "--scenario",
            "unknown-scenario",
        ])
        .output()
        .expect("run public Python journey selector");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown recovery scenario"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("passed"));
}
