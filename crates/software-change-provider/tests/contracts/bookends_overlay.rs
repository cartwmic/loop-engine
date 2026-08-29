use super::bounded_process::CommandExt;
use super::support;

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use support::{load_fixture, load_profile, provider_binary};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "software-change-bookends-overlay-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write_json(&self, name: &str, value: &Value) {
        fs::write(
            self.path.join(name),
            serde_json::to_vec(value).expect("serialize"),
        )
        .expect("write json");
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Repo {
    dir: TestDir,
}

impl Repo {
    fn new() -> Self {
        let dir = TestDir::new("repo");
        git(dir.path(), &["init", "-b", "main"]);
        git(dir.path(), &["config", "user.name", "Bookends Overlay"]);
        git(dir.path(), &["config", "user.email", "overlay@example.com"]);
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        Self { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, rel: &str, body: &str) {
        let path = self.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(path, body).expect("write");
    }

    fn commit_all(&self, message: &str) {
        git(self.path(), &["add", "-A"]);
        git(self.path(), &["commit", "-m", message]);
    }
}

fn git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

const GREEN_TOML: &str = r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]
"#;

const GREEN_WORKFLOW: &str = r#"name: ci
on: push
jobs:
  journey:
    runs-on: ubuntu-latest
    steps:
      - run: python3 tests/journey.py
"#;

fn enable_green_prd(repo: &Repo, prd: &str) {
    repo.write("bookends.toml", GREEN_TOML);
    repo.write("docs/PRD.md", prd);
    repo.write(".github/workflows/ci.yml", GREEN_WORKFLOW);
    let citation = format!("{}{}{}", "bookends", ":LE-", "1");
    repo.write("tests/journey.py", &format!("# {citation}\nprint('ok')\n"));
    repo.commit_all("enable bookends");
}

fn live_prd() -> &'static str {
    "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n"
}

fn live_and_tombstone_prd() -> &'static str {
    "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
     ### LE-2: Retired\n- Status: tombstone\n"
}

fn live_uncovered_prd() -> &'static str {
    "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
     ### LE-2: Uncovered\n- Status: live\n- Coverage: e2e/journey\n"
}

fn enable_overlay(mut profile: Value) -> Value {
    profile["extra"]["bookends"] = json!({"enabled": true});
    profile
}

fn axis_ids(policies: &Value, gate: &str) -> Vec<String> {
    policies[gate]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|axis| axis.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn run_provider_in(cwd: &Path, request: Value) -> Output {
    let mut command = Command::new(workspace_integration::binary("software-change"));
    command.current_dir(cwd);
    let bytes = serde_json::to_vec(&request).expect("serialize request");
    super::bounded_process::run_with_stdin(&mut command, "bookends overlay provider", &bytes)
        .expect("wait")
        .output
}

fn described_workflow_in(cwd: &Path, initial_input: &Value) -> Value {
    let output = run_provider_in(
        cwd,
        json!({
            "operation": "describe",
            "initial_input": initial_input
        }),
    );
    support::assert_exit(&output, 0);
    support::response(&output)
}

fn evaluate_in(cwd: &Path, initial_input: Value, transition: Value, context: Value) -> Value {
    let workflow = described_workflow_in(cwd, &initial_input);
    let output = run_provider_in(
        cwd,
        json!({
            "operation": "evaluate",
            "workflow": workflow,
            "initial_input": initial_input,
            "context": context,
            "transition": transition,
            "prior_evaluations": []
        }),
    );
    support::assert_exit(&output, 0);
    support::response(&output)
}

fn with_root(mut config: Value, root: &TestDir) -> Value {
    config["artifact_root"] = json!(root.path().to_string_lossy().to_string());
    config
}

fn intent_with_ids(ids: Value) -> Value {
    let mut intent = load_fixture("intent-good.json");
    intent["requirement_ids"] = ids;
    intent
}

fn metadata_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "revision": {"type": "string", "minLength": 1},
            "author": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 1},
                    "kind": {
                        "type": "string",
                        "enum": ["human", "agent", "script"]
                    }
                },
                "required": ["name", "kind"],
                "additionalProperties": false
            }
        },
        "required": ["revision", "author"],
        "additionalProperties": false
    })
}

fn schema_rules(value: &Value) -> Vec<String> {
    value["feedback"]["details"]["violations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|violation| {
            violation
                .get("rule")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn overlay_on_injects_requirement_ids_into_four_schemas() {
    for profile_name in ["minimal", "standard", "high-rigor"] {
        let overlayed = software_change_provider::apply_bookends_overlay_for_tests(
            &enable_overlay(load_profile(profile_name)),
        );
        for subject in [
            "intent.json",
            "design.json",
            "plan.json",
            "validation-report.json",
        ] {
            let schema = &overlayed["artifact_schemas"][subject];
            let required = schema["required"]
                .as_array()
                .expect("required")
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            assert!(
                required.contains(&"requirement_ids"),
                "{profile_name} {subject} required"
            );
            assert_eq!(schema["properties"]["requirement_ids"]["type"], "array");
            assert_eq!(schema["properties"]["requirement_ids"]["minItems"], 1);
            assert_eq!(
                schema["properties"]["requirement_ids"]["items"]["type"],
                "string"
            );
        }
        let implementation = &overlayed["artifact_schemas"]["implementation-report.json"];
        let required = implementation["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(!required.contains(&"requirement_ids"));
        assert!(implementation["properties"]
            .get("requirement_ids")
            .is_none());
    }
}

#[test]
fn overlay_on_injects_ids_grounded_on_existing_gates_and_bypass_on_validation() {
    for profile_name in ["minimal", "standard", "high-rigor"] {
        let original = load_profile(profile_name);
        let overlayed = software_change_provider::apply_bookends_overlay_for_tests(
            &enable_overlay(original.clone()),
        );
        let original_policies = &original["review_policies"];
        let policies = &overlayed["review_policies"];
        for (gate, axes) in original_policies.as_object().expect("policies") {
            let original_ids = axis_ids(original_policies, gate);
            let injected = axis_ids(policies, gate);
            if axes.as_array().is_some_and(|entries| entries.is_empty()) {
                assert!(injected.is_empty(), "{profile_name} {gate} stayed empty");
                continue;
            }
            assert!(
                injected.contains(&"ids-grounded".to_owned()),
                "{profile_name} {gate} ids-grounded"
            );
            if gate == "validation-review" || gate == "validation-adversarial-review" {
                assert!(
                    injected.contains(&"bypass-not-green".to_owned()),
                    "{profile_name} {gate} bypass-not-green"
                );
            } else {
                assert!(
                    !injected.contains(&"bypass-not-green".to_owned()),
                    "{profile_name} {gate} must not gain bypass-not-green"
                );
            }
            for id in original_ids {
                assert!(injected.contains(&id), "{profile_name} {gate} kept {id}");
            }
        }
    }
}

#[test]
fn overlay_on_empty_requirement_ids_deny() {
    let repo = Repo::new();
    enable_green_prd(&repo, live_prd());
    let artifacts = TestDir::new("empty-ids");
    artifacts.write_json("intent.json", &intent_with_ids(json!([])));
    let config = with_root(enable_overlay(load_profile("high-rigor")), &artifacts);
    let value = evaluate_in(
        repo.path(),
        config,
        support::checked("explore", "intent-ready", "intent-review"),
        json!([]),
    );
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert!(schema_rules(&value).iter().any(|rule| rule == "minItems"));
}

#[test]
fn overlay_on_missing_requirement_ids_deny() {
    let repo = Repo::new();
    enable_green_prd(&repo, live_prd());
    let artifacts = TestDir::new("missing-ids");
    artifacts.write_json("intent.json", &load_fixture("intent-good.json"));
    let config = with_root(enable_overlay(load_profile("high-rigor")), &artifacts);
    let value = evaluate_in(
        repo.path(),
        config,
        support::checked("explore", "intent-ready", "intent-review"),
        json!([]),
    );
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert!(schema_rules(&value).iter().any(|rule| rule == "required"));
}

#[test]
fn overlay_on_tombstoned_id_deny() {
    let repo = Repo::new();
    enable_green_prd(&repo, live_and_tombstone_prd());
    let artifacts = TestDir::new("tombstone");
    artifacts.write_json("intent.json", &intent_with_ids(json!(["LE-2"])));
    let config = with_root(enable_overlay(load_profile("high-rigor")), &artifacts);
    let value = evaluate_in(
        repo.path(),
        config,
        support::checked("explore", "intent-ready", "intent-review"),
        json!([]),
    );
    assert_eq!(value["feedback"]["code"], "software-change-schema-invalid");
    assert!(schema_rules(&value)
        .iter()
        .any(|rule| rule == "requirement-ids-live"));
}

fn overlay_validation_config(root: &TestDir) -> Value {
    json!({
        "config_version": "test-1",
        "artifact_root": root.path().to_string_lossy().to_string(),
        "extra": {"bookends": {"enabled": true}},
        "review_policies": {},
        "artifact_schemas": {
            "validation-report.json": metadata_schema()
        }
    })
}

fn write_checkpoints(repo: &Repo, artifacts: &TestDir) {
    for name in [
        "intent.json",
        "design.json",
        "plan.json",
        "implementation-report.json",
    ] {
        artifacts.write_json(name, &json!({"revision": "1"}));
    }
    for phase in ["implementation", "validation"] {
        let output = Command::new(provider_binary())
            .args([
                "checkpoint",
                "--phase",
                phase,
                "--artifact-root",
                artifacts.path().to_str().expect("artifact root"),
                "--working-directory",
                repo.path().to_str().expect("repository"),
            ])
            .current_dir(repo.path())
            .bounded_output("bookends overlay checkpoint")
            .expect("run checkpoint");
        assert!(
            output.status.success(),
            "checkpoint {phase} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Validation proof is admitted only after implementation proof has been
    // accepted by the provider transition that records its immutable history.
    let config = json!({
        "config_version": "test-1",
        "artifact_root": artifacts.path().to_string_lossy().to_string(),
        "review_policies": {}
    });
    let workflow = described_workflow_in(repo.path(), &config);
    let output = run_provider_in(
        repo.path(),
        json!({
            "operation": "evaluate",
            "workflow": workflow,
            "initial_input": config,
            "context": [],
            "transition": support::checked("implement", "implementation-ready", "validation"),
            "prior_evaluations": []
        }),
    );
    support::assert_exit(&output, 0);
    assert_eq!(support::response(&output), json!({"result": "allow"}));
}

fn validation_artifact(ids: Value) -> Value {
    json!({
        "revision": "1",
        "author": {"name": "owner", "kind": "human"},
        "requirement_ids": ids
    })
}

#[test]
fn overlay_on_checker_red_denies_passed() {
    let repo = Repo::new();
    enable_green_prd(&repo, live_uncovered_prd());
    let artifacts = TestDir::new("red-passed");
    artifacts.write_json(
        "validation-report.json",
        &validation_artifact(json!(["LE-1"])),
    );
    write_checkpoints(&repo, &artifacts);
    let value = evaluate_in(
        repo.path(),
        overlay_validation_config(&artifacts),
        support::checked("validation", "passed", "end"),
        json!([]),
    );
    assert_eq!(value["result"], "deny");
    assert_eq!(value["feedback"]["code"], "software-change-bookends-red");
    assert_eq!(value["feedback"]["details"]["status"], "red");
}

#[test]
fn overlay_on_checker_green_allows_schema_only_passed() {
    let repo = Repo::new();
    enable_green_prd(&repo, live_prd());
    let artifacts = TestDir::new("green-passed");
    artifacts.write_json(
        "validation-report.json",
        &validation_artifact(json!(["LE-1"])),
    );
    write_checkpoints(&repo, &artifacts);
    let value = evaluate_in(
        repo.path(),
        overlay_validation_config(&artifacts),
        support::checked("validation", "passed", "end"),
        json!([]),
    );
    assert_eq!(value, json!({"result": "allow"}));
}

fn evidence(gate: &str, axis: &str, result: &str, findings: &str, sequence: u64) -> Value {
    json!({
        "id": format!("ctx-{sequence}"),
        "kind": "review-evidence",
        "data": {
            "gate": gate,
            "policy_id": axis,
            "result": result,
            "findings": findings,
            "author": {"name": "reviewer", "kind": "agent"},
            "subject": "validation-report.json",
            "subject_revision": "r15",
            "config_version": "test-1"
        },
        "sequence": sequence,
        "created_at": sequence
    })
}

fn passing_evidence(gate: &str, axis: &str, sequence: u64) -> Value {
    evidence(gate, axis, "pass", "", sequence)
}

fn failing_evidence(gate: &str, axis: &str, sequence: u64) -> Value {
    evidence(
        gate,
        axis,
        "fail",
        "The report presents the in-process Red result as a green check.",
        sequence,
    )
}

#[test]
fn overlay_on_greenwash_fails_bypass_not_green() {
    let repo = Repo::new();
    enable_green_prd(&repo, live_uncovered_prd());
    let artifacts = TestDir::new("greenwash");
    let mut report = load_fixture("validation-report-good.json");
    report["requirement_ids"] = json!(["LE-1"]);
    report["validation"] = json!(["In-process bookends check is green and validation passed"]);
    artifacts.write_json("validation-report.json", &report);
    write_checkpoints(&repo, &artifacts);

    let schema = load_profile("high-rigor")["artifact_schemas"]["validation-report.json"].clone();
    let config = json!({
        "config_version": "test-1",
        "artifact_root": artifacts.path().to_string_lossy().to_string(),
        "extra": {"bookends": {"enabled": true}},
        "review_policies": {
            "validation-review": [{"id": "delivery", "description": "d"}],
            "validation-adversarial-review": [{"id": "delivery", "description": "d"}]
        },
        "artifact_schemas": {
            "validation-report.json": schema
        }
    });
    let context = json!([
        passing_evidence("validation-review", "delivery", 1),
        passing_evidence("validation-review", "ids-grounded", 2),
        failing_evidence("validation-review", "bypass-not-green", 3)
    ]);
    let value = evaluate_in(
        repo.path(),
        config,
        support::checked(
            "validation-review",
            "approved",
            "validation-adversarial-review",
        ),
        context,
    );
    assert_eq!(value["result"], "deny");
    assert_eq!(
        value["feedback"]["code"],
        "software-change-finding-ledger-invalid"
    );
    assert_eq!(value["feedback"]["details"]["phase"], "finding-ledger");
    assert_eq!(value["feedback"]["details"]["status"], "missing");
}
