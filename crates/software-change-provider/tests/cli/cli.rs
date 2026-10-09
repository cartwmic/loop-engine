use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn invoke(arguments: &[&str], stdin: &[u8]) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    command.args(arguments);
    let completed =
        super::bounded_process::run_with_stdin(&mut command, "software-change CLI protocol", stdin)
            .expect("software-change process should exit");
    if let Some(error) = completed.stdin_error {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::BrokenPipe,
            "unexpected stdin write failure: {error}"
        );
    }
    completed.output
}

#[test]
fn help_flags_return_conventional_stdout_without_reading_protocol() {
    for flag in ["--help", "-h"] {
        let output = invoke(&[flag], b"not protocol JSON");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        let help = String::from_utf8_lossy(&output.stdout);
        assert!(help.contains("software-change"));
        assert!(help.contains("describe"));
        assert!(help.contains("evaluate"));
        assert!(help.contains("data-dump"));
        assert!(!help.contains("run-plan-graph"));
        assert!(help.contains("review-candidates"));
    }
}

#[test]
fn version_flags_report_workspace_package_version_on_stdout() {
    for flag in ["--version", "-V"] {
        let output = invoke(&[flag], b"not protocol JSON");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("software-change {}\n", env!("CARGO_PKG_VERSION")),
            "flag={flag}"
        );
    }
}

#[test]
fn no_arguments_keep_stdin_protocol_and_unsupported_arguments_keep_error_taxonomy() {
    let describe = invoke(&[], br#"{"operation":"describe"}"#);
    assert_eq!(describe.status.code(), Some(0));
    assert!(!describe.stdout.is_empty());

    let unsupported = invoke(&["unknown"], b"");
    assert_eq!(unsupported.status.code(), Some(2));
    assert!(unsupported.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("unsupported command"));
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn candidate_temp_dir() -> PathBuf {
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "software-change-review-candidates-cli-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("candidate temp directory");
    path
}

#[test]
fn review_candidates_cli_preserves_original_and_isolates_malformed_siblings() {
    let input = json!({"show":{"operation":"show","status":"completed","result":{"workflow_id":"software-change","initial_input":{"config_version":"minimal-15","review_policies":{"intent-review":[{"id":"intent-sufficient","review_stage":"aggregate","group":"intent-review.intent-sufficient"}]}},"context":[]}},"assignment":{"gate":"intent-review","stage":"aggregate","group":"intent-review.intent-sufficient","author":{"name":"external-reviewer","kind":"agent"},"subject":"intent.json","subject_revision":"1"},"request":{"text":"actual review instructions"},"original":{"text":"A prose response."},"projection":{"judgments":[{"axis":"intent-sufficient","result":"pass","findings":"","grounds":{"reason":"reconstructed final state","evidence":[{"locator":"intent.json#/goal"}]}},false,{"axis":"intent-sufficient","result":"pass"}]}});
    let output = invoke(&["review-candidates"], &serde_json::to_vec(&input).unwrap());
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["records"].as_array().unwrap().len(), 1);
    assert_eq!(
        document["records"][0]["data"]["external_review"]["original"]["original"],
        "A prose response."
    );
    assert!(document["items"][1]["candidate"].is_null());
    assert!(document["items"][2]["candidate"].is_null());
}

#[test]
fn review_candidates_cli_rejects_non_show_input() {
    let output = invoke(&["review-candidates"], br#"{"operation":"evaluate"}"#);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("completed software-change full show"));
}

#[test]
fn review_candidates_cli_rejects_foreign_workflow() {
    let input = json!({
        "operation": "show",
        "status": "completed",
        "result": {"workflow_id": "research"}
    });
    let output = invoke(
        &["review-candidates"],
        &serde_json::to_vec(&input).expect("show JSON"),
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("software-change"));
}

#[test]
fn external_validation_packet_accepts_results_and_fails_closed() {
    let repo = candidate_temp_dir();
    let artifacts = candidate_temp_dir();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    let spec = json!({"id":"proof","command":"driver-proof","args":["check"],"owner":"driver","obligation":"observable outcome"});
    fs::write(
        artifacts.join("plan.json"),
        json!({"revision":"r1","tasks":[],"proof_commands":[spec]}).to_string(),
    )
    .unwrap();
    fs::write(
        artifacts.join("intent.json"),
        json!({"acceptance":[{"id":"AC-1","statement":"outcome"}]}).to_string(),
    )
    .unwrap();
    fs::write(
        artifacts.join("implementation-report.json"),
        json!({"revision":"i1","author":{"name":"implementer","kind":"agent"}}).to_string(),
    )
    .unwrap();
    let identity = invoke(
        &[
            "source-identity",
            "--working-directory",
            repo.to_str().unwrap(),
        ],
        b"",
    );
    assert!(identity.status.success(), "{:?}", identity.stderr);
    let source: Value = serde_json::from_slice(&identity.stdout).unwrap();
    let describe = invoke(&[], br#"{"operation":"describe"}"#);
    let graph: Value = serde_json::from_slice(&describe.stdout).unwrap();
    let result = json!({"id":"proof","command":"driver-proof","args":["check"],"cwd":repo,"exit_code":0,"stdout":"proof passed","producer":{"name":"external-driver","kind":"script"},"source":source});
    let mut packet = json!({"working_directory":repo,"revision":"v1","verdict_prefix":"verdict","author":{"name":"proof-owner","kind":"human"},"command_results":[result],"show":{"operation":"show","status":"completed","result":{"current_state":"validation","work_slots":graph["work_slots"],"context":[],"initial_input":{"artifact_root":artifacts,"contract_version":3,"criterion_policy":{"required_authors":1,"goal_required_authors":1}}}}});
    let prepare = |packet: &Value| {
        let output = invoke(
            &["prepare-validation"],
            &serde_json::to_vec(packet).unwrap(),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let success = prepare(&packet);
    assert_eq!(success["commands_complete"], true);
    assert_eq!(
        success["command_candidates"][0]["data"]["external_result"]["external"],
        true
    );
    assert!(success["command_candidates"][0]["data"]
        .get("origin")
        .is_none());
    assert_eq!(
        success["command_candidates"][0]["data"]["external_result"],
        {
            let mut result = packet["command_results"][0].clone();
            result["external"] = json!(true);
            result
        }
    );
    assert!(success["command_candidates"][0]["data"]
        .get("capture")
        .is_none());
    packet["command_results"][0]["exit_code"] = json!(7);
    assert_eq!(prepare(&packet)["diagnostics"][0]["status"], "failed");
    packet["command_results"][0]["exit_code"] = json!(0);
    packet["command_results"][0]["source"]["head"] = json!("stale");
    assert_eq!(prepare(&packet)["diagnostics"][0]["status"], "stale-source");
    packet["command_results"][0]["command"] = json!("different");
    assert_eq!(
        invoke(
            &["prepare-validation"],
            &serde_json::to_vec(&packet).unwrap()
        )
        .status
        .code(),
        Some(2)
    );
    fs::remove_dir_all(repo).unwrap();
    fs::remove_dir_all(artifacts).unwrap();
}

#[test]
fn setup_is_policy_only_and_retired_commands_refuse() {
    let root = candidate_temp_dir();
    let output = root.join("profile.json");
    let setup = invoke(
        &[
            "setup",
            "--rigor",
            "standard",
            "--output",
            output.to_str().unwrap(),
            "--enable-advice",
        ],
        b"",
    );
    assert!(setup.status.success(), "{:?}", setup.stderr);
    let profile: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert!(profile.get("work_slot_bindings").is_none());
    assert!(profile.get("advice_command").is_none());
    assert_eq!(profile["extra"]["advice"]["enabled"], true);
    assert_eq!(
        serde_json::from_slice::<Value>(&setup.stdout).unwrap(),
        profile
    );
    for command in [
        "run-plan-graph",
        "run-validation",
        "validation-command",
        "stdin-exec",
    ] {
        assert_eq!(invoke(&[command], b"").status.code(), Some(2));
    }
    for flag in [
        "--roster",
        "--engine",
        "--provider",
        "--advice-config",
        "--implementation",
    ] {
        assert_eq!(
            invoke(&["setup", flag, "/bin/true"], b"").status.code(),
            Some(2)
        );
    }
    fs::remove_dir_all(root).unwrap();
}
