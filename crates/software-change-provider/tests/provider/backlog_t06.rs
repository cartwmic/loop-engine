use super::bounded_process::CommandExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_root(label: &str) -> PathBuf {
    let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "software-change-backlog-t06-{label}-{}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("temporary root");
    path
}

fn provider() -> PathBuf {
    workspace_integration::binary("software-change")
}

fn engine() -> PathBuf {
    workspace_integration::binary("loop-engine")
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).expect("worker script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("worker metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("worker permissions");
    }
}

fn worker_body() -> &'static str {
    r##"#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path

packet = sys.stdin.read()
def value(prefix, default):
    return next((line[len(prefix):] for line in packet.splitlines() if line.startswith(prefix)), default)

policies = json.loads(value("assigned_policies: ", "[]"))
stage = value("review_stage: ", "aggregate")
author = value("required_author_claim: ", "unknown")
log = Path(sys.argv[1])
with log.open("a", encoding="utf-8") as stream:
    stream.write(json.dumps({
        "command": sys.argv[0],
        "argv": sys.argv[1:],
        "session_dir": os.environ.get("PI_CODING_AGENT_SESSION_DIR"),
        "stage": stage,
        "author": author,
        "stdin": packet,
    }) + "\n")
print(json.dumps({
    "review_stage": stage,
    "author": {"name": author, "kind": "agent"},
    "judgments": [
        {"axis": policy["id"], "result": "pass", "findings": ""}
        for policy in policies
    ],
}))
"##
}

fn setup(
    root: &Path,
    rigor: &str,
    roster: &Path,
    output: &Path,
    extra: &[&str],
) -> (Output, Value) {
    setup_with_provider(&provider(), root, rigor, roster, output, extra)
}

fn setup_with_provider(
    provider_binary: &Path,
    root: &Path,
    rigor: &str,
    roster: &Path,
    output: &Path,
    extra: &[&str],
) -> (Output, Value) {
    let mut command = Command::new(provider_binary);
    command.current_dir(root);
    command.args([
        "setup",
        "--rigor",
        rigor,
        "--roster",
        roster.to_str().expect("roster path"),
        "--engine",
        engine().to_str().expect("engine path"),
        "--provider",
        provider_binary.to_str().expect("provider path"),
        "--output",
        output.to_str().expect("output path"),
    ]);
    command.args(extra);
    let completed = command
        .bounded_output("software-change setup backlog_t06")
        .expect("setup process");
    let report = if completed.stdout.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&completed.stdout).unwrap_or_else(|error| {
            panic!(
                "setup report must be JSON: {error}: {}",
                String::from_utf8_lossy(&completed.stdout)
            )
        })
    };
    (completed, report)
}

fn worker_values(binding: &Value) -> Vec<Value> {
    let args = binding["args"].as_array().expect("binding args");
    let mut workers = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--worker" {
            workers.push(
                serde_json::from_str(args[index + 1].as_str().expect("worker JSON"))
                    .expect("worker object"),
            );
            index += 2;
        } else {
            index += 1;
        }
    }
    workers
}

fn expected_worker_count(policies: &Value, roster_len: usize) -> usize {
    let policies = policies.as_array().expect("policy array");
    let has_individual = policies
        .iter()
        .any(|policy| policy.get("review_stage").and_then(Value::as_str) == Some("individual"));
    let mut count = 0;
    if has_individual {
        count += policies
            .iter()
            .filter(|policy| {
                policy.get("review_stage").and_then(Value::as_str) == Some("individual")
            })
            .map(|policy| {
                policy["required_authors"]
                    .as_u64()
                    .expect("required authors") as usize
            })
            .sum::<usize>();
    }
    for author_index in 0..roster_len {
        if policies.iter().any(|policy| {
            policy
                .get("review_stage")
                .and_then(Value::as_str)
                .unwrap_or("aggregate")
                == "aggregate"
                && policy["required_authors"]
                    .as_u64()
                    .expect("required authors") as usize
                    > author_index
        }) {
            count += 1;
        }
    }
    count
}

fn assigned_policy_ids(worker: &Value) -> Vec<String> {
    let raw = worker["preamble"]
        .as_str()
        .expect("worker preamble")
        .lines()
        .find_map(|line| line.strip_prefix("assigned_policies: "))
        .expect("assigned policies");
    serde_json::from_str::<Vec<Value>>(raw)
        .expect("assigned policies JSON")
        .into_iter()
        .map(|policy| policy["id"].as_str().expect("policy id").to_owned())
        .collect()
}

fn worker_stage(worker: &Value) -> &str {
    worker["full_output_schema"]["properties"]["review_stage"]["const"]
        .as_str()
        .expect("worker stage")
}

fn expected_launches(policies: &Value, authors: &[&str]) -> Vec<(String, String)> {
    let policies = policies.as_array().expect("policy array");
    let has_individual = policies
        .iter()
        .any(|policy| policy.get("review_stage").and_then(Value::as_str) == Some("individual"));
    let mut launches = Vec::new();
    if has_individual {
        for (author_index, author) in authors.iter().enumerate() {
            for policy in policies.iter().filter(|policy| {
                policy.get("review_stage").and_then(Value::as_str) == Some("individual")
                    && policy["required_authors"]
                        .as_u64()
                        .expect("required authors") as usize
                        > author_index
            }) {
                let _ = policy;
                launches.push(("individual".to_owned(), (*author).to_owned()));
            }
        }
    }
    for (author_index, author) in authors.iter().enumerate() {
        if policies.iter().any(|policy| {
            policy
                .get("review_stage")
                .and_then(Value::as_str)
                .unwrap_or("aggregate")
                == "aggregate"
                && policy["required_authors"]
                    .as_u64()
                    .expect("required authors") as usize
                    > author_index
        }) {
            launches.push(("aggregate".to_owned(), (*author).to_owned()));
        }
    }
    launches
}

fn assert_setup_output(
    output: &Path,
    report: &Value,
    rigor: &str,
    bookends: bool,
    roster_len: usize,
    provider_binary: &Path,
) -> Value {
    assert_eq!(report["status"], "ready");
    assert_eq!(report["rigor"], rigor);
    assert_eq!(report["bookends_enabled"], bookends);
    assert_eq!(report["started"], false);
    assert_eq!(
        report["preview"]
            .get("errors")
            .cloned()
            .unwrap_or_else(|| json!([])),
        json!([])
    );
    assert_eq!(report["output_path"], output.to_string_lossy().as_ref());

    let bytes = fs::read(output).expect("generated profile");
    let text = String::from_utf8(bytes.clone()).expect("profile UTF-8");
    let digest = format!("{:x}", Sha256::digest(&bytes));
    assert_eq!(report["output_bytes"], text);
    assert_eq!(report["output_byte_length"], bytes.len());
    assert_eq!(report["output_sha256"], digest);
    assert_eq!(report["output_sha256_digest"], format!("sha256:{digest}"));

    let profile: Value = serde_json::from_slice(&bytes).expect("profile JSON");
    assert_eq!(profile["contract_version"], 3);
    assert_eq!(
        profile["config_version"],
        format!("{rigor}-10").replace("high-10", "high-rigor-10")
    );
    assert_eq!(
        profile["work_slot_bindings"].as_object().unwrap().len(),
        report["effective_policy"]["binding_slots"]
            .as_array()
            .expect("binding slots")
            .len()
    );
    if bookends {
        assert_eq!(profile["extra"]["bookends"]["enabled"], true);
    } else {
        assert!(profile["extra"]["bookends"].is_null());
    }

    for gate in [
        "intent-review",
        "intent-adversarial-review",
        "design-review",
        "design-adversarial-review",
        "plan-review",
        "plan-adversarial-review",
        "implementation-review",
        "implementation-adversarial-review",
        "validation-review",
        "validation-adversarial-review",
    ] {
        let binding = &profile["work_slot_bindings"][gate];
        assert_eq!(binding["command"], engine().to_string_lossy().as_ref());
        assert_eq!(
            binding["context_filter"],
            json!({
                "command": provider_binary.to_string_lossy(),
                "args": ["commission"]
            })
        );
        let args = binding["args"].as_array().expect("fan-out args");
        assert_eq!(args[0], "fan-out");
        assert_eq!(args[1], "--max-active");
        assert_eq!(args[2], "2");
        assert_eq!(
            worker_values(binding).len(),
            expected_worker_count(
                &report["effective_policy"]["review_policies"][gate],
                roster_len,
            )
        );
        assert!(!args.iter().any(|arg| arg == "--model"));
        for worker in worker_values(binding) {
            assert!(worker["preamble"]
                .as_str()
                .unwrap()
                .contains("FROZEN REVIEW ASSIGNMENT"));
            assert!(worker["full_output_schema"].is_object());
            assert!(worker.get("output_schema").is_none());
        }
    }
    profile
}

#[test]
fn backlog_t06_setup_profiles_are_inspectable_and_closed() {
    let root = temp_root("profiles");
    let worker_a = root.join("worker-a.py");
    let worker_b = root.join("worker-b.py");
    write_executable(&worker_a, worker_body());
    write_executable(&worker_b, worker_body());
    let roster = root.join("roster.json");
    fs::write(
        &roster,
        serde_json::to_vec(&json!([
            {"author":"reviewer-a","command":worker_a,"args":["literal-a","--effort"]},
            {"author":"reviewer-b","command":worker_b,"args":["literal-b"]}
        ]))
        .expect("roster JSON"),
    )
    .expect("roster");

    for rigor in ["minimal", "standard", "high"] {
        let output = root.join(format!("{rigor}.json"));
        let (process, report) = setup(&root, rigor, &roster, &output, &[]);
        assert_eq!(process.status.code(), Some(0), "setup {rigor}: {process:?}");
        assert_setup_output(&output, &report, rigor, false, 2, &provider());
        let profile: Value =
            serde_json::from_slice(&fs::read(&output).expect("profile")).expect("profile JSON");
        let worker = &worker_values(&profile["work_slot_bindings"]["intent-review"])[0];
        assert_eq!(worker["command"], worker_a.to_string_lossy().as_ref());
        assert_eq!(worker["args"], json!(["literal-a", "--effort"]));
        assert_eq!(report["preview"]["models"], json!([]));
        if rigor == "high" {
            assert!(profile["work_slot_bindings"]["intent-review"]["args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|arg| arg == "--then"));
            let workers = worker_values(&profile["work_slot_bindings"]["intent-review"]);
            let individual: Vec<_> = workers
                .iter()
                .filter(|worker| {
                    worker["full_output_schema"]["properties"]["review_stage"]["const"]
                        == "individual"
                })
                .collect();
            let aggregate: Vec<_> = workers
                .iter()
                .filter(|worker| {
                    worker["full_output_schema"]["properties"]["review_stage"]["const"]
                        == "aggregate"
                })
                .collect();
            let individual_axes = report["effective_policy"]["review_policies"]["intent-review"]
                .as_array()
                .expect("intent policies")
                .iter()
                .filter(|policy| policy["review_stage"] == "individual")
                .count();
            assert_eq!(individual.len(), individual_axes * 2);
            assert_eq!(aggregate.len(), 2);
            for worker in individual {
                let judgments = &worker["full_output_schema"]["properties"]["judgments"];
                assert_eq!(judgments["minItems"], 1);
                assert_eq!(judgments["maxItems"], 1);
                assert_eq!(
                    judgments["items"]["oneOf"][0]["properties"]["axis"]["enum"]
                        .as_array()
                        .expect("individual axis enum")
                        .len(),
                    1
                );
            }
            for worker in aggregate {
                assert!(worker["preamble"]
                    .as_str()
                    .expect("aggregate preamble")
                    .contains("fresh independent reviewer session"));
                assert!(worker["preamble"]
                    .as_str()
                    .expect("aggregate preamble")
                    .contains("individual-stage captures"));
            }
        }
    }

    let work = root.join("implementation-work");
    fs::create_dir(&work).expect("implementation work directory");
    let implementation = root.join("implementation.json");
    fs::write(
        &implementation,
        serde_json::to_vec(&json!({
            "command": worker_b,
            "args": ["implementation-literal"],
            "working_directory": work,
        }))
        .expect("implementation JSON"),
    )
    .expect("implementation");
    let bookends_output = root.join("high-bookends.json");
    let (process, report) = setup(
        &root,
        "high",
        &roster,
        &bookends_output,
        &[
            "--bookends",
            "--implementation",
            implementation.to_str().expect("implementation path"),
        ],
    );
    assert_eq!(
        process.status.code(),
        Some(0),
        "bookends setup: {process:?}"
    );
    let profile = assert_setup_output(&bookends_output, &report, "high", true, 2, &provider());
    assert!(profile["work_slot_bindings"].get("implement").is_some());
    let implementation_args = profile["work_slot_bindings"]["implement"]["args"]
        .as_array()
        .expect("implementation args");
    assert_eq!(implementation_args[0], "run-plan-graph");
    assert_eq!(implementation_args[1], "--working-directory");
    assert_eq!(implementation_args[2], work.to_string_lossy().as_ref());
    assert_eq!(implementation_args[3], "--max-active");
    assert_eq!(implementation_args[4], "1");
    assert_eq!(implementation_args[5], "--task-worker");
    let task_worker: Value = serde_json::from_str(
        profile["work_slot_bindings"]["implement"]["args"][6]
            .as_str()
            .expect("task worker JSON"),
    )
    .expect("task worker");
    assert_eq!(task_worker["command"], worker_b.to_string_lossy().as_ref());
    assert_eq!(task_worker["args"], json!(["implementation-literal"]));
    assert!(
        worker_values(&profile["work_slot_bindings"]["intent-review"])
            .iter()
            .flat_map(|worker| {
                worker["full_output_schema"]["properties"]["judgments"]["items"]["oneOf"][0]
                    ["properties"]["axis"]["enum"]
                    .as_array()
                    .into_iter()
                    .flatten()
            })
            .any(|axis| axis == "ids-grounded")
    );

    let sentinel = root.join("sentinel.json");
    fs::write(&sentinel, b"keep this exact file").expect("sentinel");
    let duplicate = root.join("duplicate-roster.json");
    fs::write(
        &duplicate,
        br#"[{"author":"reviewer-a","command":"/bin/true","args":[]},{"author":"reviewer-a","command":"/bin/false","args":[]}]"#,
    )
    .expect("duplicate roster");
    let (process, _) = setup(&root, "standard", &duplicate, &sentinel, &[]);
    assert_ne!(process.status.code(), Some(0));
    assert_eq!(
        fs::read(&sentinel).expect("sentinel bytes"),
        b"keep this exact file"
    );

    let malformed = root.join("malformed-roster.json");
    fs::write(
        &malformed,
        br#"[{"author":"reviewer-a","command":"/bin/true","args":[],"model":"must-not-be-adapted"}]"#,
    )
    .expect("malformed roster");
    let (process, _) = setup(&root, "standard", &malformed, &sentinel, &[]);
    assert_ne!(process.status.code(), Some(0));
    assert_eq!(
        fs::read(&sentinel).expect("malformed sentinel bytes"),
        b"keep this exact file"
    );

    let short = root.join("short-roster.json");
    fs::write(
        &short,
        br#"[{"author":"only","command":"/bin/true","args":[]}]"#,
    )
    .expect("short roster");
    let (process, _) = setup(&root, "standard", &short, &root.join("short.json"), &[]);
    assert_ne!(process.status.code(), Some(0));

    let invalid_implementation = root.join("invalid-implementation.json");
    fs::write(
        &invalid_implementation,
        serde_json::to_vec(&json!({
            "command": worker_b,
            "args": ["contains\nline-break"],
            "working_directory": work,
        }))
        .expect("invalid implementation JSON"),
    )
    .expect("invalid implementation");
    let implementation_sentinel = root.join("implementation-sentinel.json");
    fs::write(&implementation_sentinel, b"keep implementation output")
        .expect("implementation sentinel");
    let (process, _) = setup(
        &root,
        "standard",
        &roster,
        &implementation_sentinel,
        &[
            "--implementation",
            invalid_implementation
                .to_str()
                .expect("implementation path"),
        ],
    );
    assert_ne!(process.status.code(), Some(0));
    assert_eq!(
        fs::read(&implementation_sentinel).expect("implementation sentinel bytes"),
        b"keep implementation output"
    );
}

fn engine_call(database: &Path, args: &[String]) -> Value {
    let mut command = Command::new(engine());
    command
        .arg("--database")
        .arg(database)
        .arg("--json")
        .args(args);
    let completed = command
        .bounded_output("loop-engine backlog_t06")
        .expect("engine process");
    serde_json::from_slice(&completed.stdout).unwrap_or_else(|error| {
        panic!(
            "engine output must be JSON: {error}: {}",
            String::from_utf8_lossy(&completed.stdout)
        )
    })
}

#[test]
fn backlog_t06_relocated_embedded_setup_executes_distinct_scripted_workers() {
    let root = temp_root("embedded");
    let relocated_provider = root.join("software-change");
    fs::copy(provider(), &relocated_provider).expect("relocated provider");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&relocated_provider)
            .expect("relocated provider metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&relocated_provider, permissions).expect("relocated provider mode");
    }

    let worker_a = root.join("embedded-worker-a.py");
    let worker_b = root.join("embedded-worker-b.py");
    let log_a = root.join("embedded-worker-a.jsonl");
    let log_b = root.join("embedded-worker-b.jsonl");
    write_executable(&worker_a, worker_body());
    write_executable(&worker_b, worker_body());
    let roster = root.join("roster.json");
    fs::write(
        &roster,
        serde_json::to_vec(&json!([
            {"author":"embedded-a","command":worker_a,"args":[log_a,"--literal-a"]},
            {"author":"embedded-b","command":worker_b,"args":[log_b,"--literal-b"]}
        ]))
        .expect("roster JSON"),
    )
    .expect("roster");
    let profile_path = root.join("profile.json");
    let (setup_process, report) = setup_with_provider(
        &relocated_provider,
        &root,
        "high",
        &roster,
        &profile_path,
        &[],
    );
    assert_eq!(
        setup_process.status.code(),
        Some(0),
        "setup: {setup_process:?}"
    );
    let profile = assert_setup_output(
        &profile_path,
        &report,
        "high",
        false,
        2,
        &relocated_provider,
    );
    assert!(
        !root.join("loop.sqlite").exists(),
        "setup must not create a run database"
    );
    assert_eq!(
        profile["work_slot_bindings"]["intent-review"]["context_filter"],
        json!({"command": relocated_provider.to_string_lossy(), "args": ["commission"]})
    );
    for worker in worker_values(&profile["work_slot_bindings"]["intent-review"]) {
        assert!(
            worker["command"] == worker_a.to_string_lossy().as_ref()
                || worker["command"] == worker_b.to_string_lossy().as_ref(),
            "unexpected embedded worker command: {worker}"
        );
    }

    let providers = root.join("providers.toml");
    fs::write(
        &providers,
        format!(
            "[providers.software-change]\ncommand = {:?}\nargs = []\n",
            relocated_provider.to_string_lossy()
        ),
    )
    .expect("provider catalog");
    let database = root.join("execution.sqlite");
    let run_id = "backlog-t06-embedded";
    let start = engine_call(
        &database,
        &[
            "--config".into(),
            providers.to_string_lossy().into_owned(),
            "start".into(),
            "--id".into(),
            run_id.into(),
            "software-change".into(),
            format!("@{}", profile_path.display()),
            "backlog t06 embedded".into(),
        ],
    );
    assert_eq!(start["status"], "completed", "start: {start}");
    let artifact_root = PathBuf::from(
        start["result"]["run"]["initial_input"]["artifact_root"]
            .as_str()
            .expect("artifact root"),
    );
    fs::create_dir_all(&artifact_root).expect("artifact root");
    let fixture = workspace_integration::package_root("software-change-provider")
        .join("data/calibration/fixtures/intent-good.json");
    fs::copy(fixture, artifact_root.join("intent.json")).expect("intent fixture");
    assert_eq!(
        engine_call(
            &database,
            &["show".into(), "--view".into(), "full".into(), run_id.into()]
        )["status"],
        "completed"
    );
    assert_eq!(
        engine_call(
            &database,
            &["event".into(), run_id.into(), "intent-ready".into()]
        )["status"],
        "completed"
    );
    assert_eq!(
        engine_call(
            &database,
            &["show".into(), "--view".into(), "full".into(), run_id.into()]
        )["status"],
        "completed"
    );
    let invoke = engine_call(
        &database,
        &["invoke".into(), run_id.into(), "intent-review".into()],
    );
    assert_eq!(invoke["status"], "completed", "invoke: {invoke}");
    let invocation_id = invoke["result"]["invocation_id"]
        .as_str()
        .expect("invocation ID")
        .to_owned();
    let final_show = (0..240)
        .find_map(|_| {
            let shown = engine_call(
                &database,
                &["show".into(), "--view".into(), "full".into(), run_id.into()],
            );
            let invocation = shown["result"]["work_slot_invocations"]
                .as_array()
                .expect("invocations")
                .iter()
                .find(|row| row["invocation_id"] == invocation_id)?;
            match invocation["status"].as_str() {
                Some("succeeded") | Some("failed") => Some(shown),
                _ => {
                    thread::sleep(Duration::from_millis(50));
                    None
                }
            }
        })
        .expect("embedded invocation completion");
    let invocation = final_show["result"]["work_slot_invocations"]
        .as_array()
        .expect("invocations")
        .iter()
        .find(|row| row["invocation_id"] == invocation_id)
        .expect("selected invocation");
    assert_eq!(
        invocation["status"], "succeeded",
        "embedded invocation: {invocation}"
    );
    let expected = expected_launches(
        &report["effective_policy"]["review_policies"]["intent-review"],
        &["embedded-a", "embedded-b"],
    );
    assert_eq!(
        invocation["inner_workers"]
            .as_array()
            .expect("inner workers")
            .len(),
        expected.len()
    );
    let mut launches = Vec::new();
    for log in [&log_a, &log_b] {
        if log.is_file() {
            launches.extend(
                fs::read_to_string(log)
                    .expect("worker log")
                    .lines()
                    .map(|line| serde_json::from_str::<Value>(line).expect("worker log JSON")),
            );
        }
    }
    let observed = launches
        .iter()
        .map(|launch| {
            (
                launch["stage"].as_str().expect("stage").to_owned(),
                launch["author"].as_str().expect("author").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(observed.len(), expected.len());
    for assignment in expected {
        assert!(observed.contains(&(assignment.0, assignment.1)));
    }
    for launch in launches {
        let stdin = launch["stdin"].as_str().expect("stdin");
        assert!(stdin.contains("artifact_root"));
        let argv = launch["argv"].as_array().expect("worker argv");
        if argv[0] == log_a.to_string_lossy().as_ref() {
            assert!(argv.iter().any(|argument| argument == "--literal-a"));
            assert_eq!(launch["command"], worker_a.to_string_lossy().as_ref());
        } else {
            assert_eq!(argv[0], log_b.to_string_lossy().as_ref());
            assert!(argv.iter().any(|argument| argument == "--literal-b"));
            assert_eq!(launch["command"], worker_b.to_string_lossy().as_ref());
        }
        assert!(launch["session_dir"]
            .as_str()
            .is_some_and(|session| !session.is_empty()));
    }
}

#[test]
fn backlog_t06_setup_drives_each_rigor_stage_with_distinct_scripted_workers() {
    let root = temp_root("execution");
    let provider_config = root.join("providers.toml");
    fs::write(
        &provider_config,
        format!(
            "[providers.software-change]\ncommand = {:?}\nargs = []\n",
            provider().to_string_lossy()
        ),
    )
    .expect("provider config");

    for rigor in ["minimal", "standard", "high"] {
        let run_root = root.join(rigor);
        fs::create_dir(&run_root).expect("run root");
        let worker_a = run_root.join("worker-a.py");
        let worker_b = run_root.join("worker-b.py");
        let log_a = run_root.join("worker-a.jsonl");
        let log_b = run_root.join("worker-b.jsonl");
        write_executable(&worker_a, worker_body());
        write_executable(&worker_b, worker_body());
        let roster = run_root.join("roster.json");
        fs::write(
            &roster,
            serde_json::to_vec(&json!([
                {"author":"reviewer-a","command":worker_a,"args":[log_a]},
                {"author":"reviewer-b","command":worker_b,"args":[log_b]}
            ]))
            .expect("roster JSON"),
        )
        .expect("roster");
        let profile = run_root.join("profile.json");
        let (process, report) = setup(&run_root, rigor, &roster, &profile, &[]);
        assert_eq!(process.status.code(), Some(0), "setup {rigor}: {process:?}");
        assert_setup_output(&profile, &report, rigor, false, 2, &provider());
        let generated_profile: Value =
            serde_json::from_slice(&fs::read(&profile).expect("generated profile"))
                .expect("generated profile JSON");
        let expected = expected_launches(
            &report["effective_policy"]["review_policies"]["intent-review"],
            &["reviewer-a", "reviewer-b"],
        );

        let database = run_root.join("loop.sqlite");
        let start = engine_call(
            &database,
            &[
                "--config".to_owned(),
                provider_config.to_string_lossy().into_owned(),
                "start".to_owned(),
                "--id".to_owned(),
                format!("backlog-t06-{rigor}"),
                "software-change".to_owned(),
                format!("@{}", profile.display()),
                format!("backlog t06 {rigor}"),
            ],
        );
        assert_eq!(start["status"], "completed", "start {rigor}: {start}");
        let run_id = format!("backlog-t06-{rigor}");
        let artifact_root = PathBuf::from(
            start["result"]["run"]["initial_input"]["artifact_root"]
                .as_str()
                .expect("artifact root"),
        );
        fs::create_dir_all(&artifact_root).expect("artifact root");
        let fixture = workspace_integration::package_root("software-change-provider")
            .join("data/calibration/fixtures/intent-good.json");
        fs::copy(fixture, artifact_root.join("intent.json")).expect("intent fixture");

        assert_eq!(
            engine_call(
                &database,
                &[
                    "show".into(),
                    "--view".into(),
                    "full".into(),
                    run_id.clone()
                ]
            )["status"],
            "completed"
        );
        assert_eq!(
            engine_call(
                &database,
                vec!["event".into(), run_id.clone(), "intent-ready".to_owned()].as_slice()
            )["status"],
            "completed"
        );
        assert_eq!(
            engine_call(
                &database,
                &[
                    "show".into(),
                    "--view".into(),
                    "full".into(),
                    run_id.clone()
                ]
            )["status"],
            "completed"
        );
        let invoke = engine_call(
            &database,
            &["invoke".into(), run_id.clone(), "intent-review".into()],
        );
        assert_eq!(invoke["status"], "completed", "invoke {rigor}: {invoke}");
        let invocation_id = invoke["result"]["invocation_id"]
            .as_str()
            .expect("invocation ID");

        let final_show = (0..120)
            .find_map(|_| {
                let shown = engine_call(
                    &database,
                    &[
                        "show".into(),
                        "--view".into(),
                        "full".into(),
                        run_id.clone(),
                    ],
                );
                let invocation = shown["result"]["work_slot_invocations"]
                    .as_array()
                    .expect("invocations")
                    .iter()
                    .find(|row| row["invocation_id"] == invocation_id)?;
                match invocation["status"].as_str() {
                    Some("succeeded") | Some("failed") => Some(shown),
                    _ => {
                        thread::sleep(Duration::from_millis(50));
                        None
                    }
                }
            })
            .expect("invocation completion");
        let invocation = final_show["result"]["work_slot_invocations"]
            .as_array()
            .expect("invocations")
            .iter()
            .find(|row| row["invocation_id"] == invocation_id)
            .expect("selected invocation");
        assert_eq!(
            invocation["status"], "succeeded",
            "invocation {rigor}: {invocation}"
        );
        assert_eq!(
            invocation["inner_workers"].as_array().unwrap().len(),
            expected.len()
        );

        let mut launches = Vec::new();
        for log in [&log_a, &log_b] {
            if log.is_file() {
                for line in fs::read_to_string(log).expect("worker log").lines() {
                    launches.push(serde_json::from_str::<Value>(line).expect("worker log JSON"));
                }
            }
        }
        let observed: Vec<_> = launches
            .iter()
            .map(|launch| {
                (
                    launch["stage"].as_str().unwrap().to_owned(),
                    launch["author"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(observed.len(), expected.len(), "worker executions {rigor}");
        for (stage, author) in expected {
            assert!(observed
                .iter()
                .any(|row| row == &(stage.to_owned(), author.to_owned())));
        }
        for launch in &launches {
            let stdin = launch["stdin"].as_str().unwrap();
            assert!(stdin.contains("FROZEN REVIEW ASSIGNMENT"));
            assert!(stdin.contains("artifact_root"));
            let argv = launch["argv"].as_array().expect("worker argv");
            if argv[0] == log_a.to_string_lossy().as_ref() {
                assert_eq!(launch["command"], worker_a.to_string_lossy().as_ref());
            } else {
                assert_eq!(argv[0], log_b.to_string_lossy().as_ref());
                assert_eq!(launch["command"], worker_b.to_string_lossy().as_ref());
            }
            assert!(launch["session_dir"]
                .as_str()
                .is_some_and(|session| !session.is_empty()));
        }

        if rigor == "high" {
            let workers = worker_values(&generated_profile["work_slot_bindings"]["intent-review"]);
            let affected_axis = workers
                .iter()
                .find(|worker| worker_stage(worker) == "individual")
                .and_then(|worker| assigned_policy_ids(worker).into_iter().next())
                .expect("affected individual axis");
            let mut selected_ids = workers
                .iter()
                .enumerate()
                .filter(|(_, worker)| {
                    let assigned = assigned_policy_ids(worker);
                    worker_stage(worker) == "individual"
                        && assigned.len() == 1
                        && assigned[0] == affected_axis
                })
                .map(|(index, _)| format!("worker-{index}"))
                .collect::<Vec<_>>();
            selected_ids.extend(
                workers
                    .iter()
                    .enumerate()
                    .filter(|(_, worker)| worker_stage(worker) == "aggregate")
                    .map(|(index, _)| format!("worker-{index}")),
            );
            assert_eq!(
                selected_ids.len(),
                4,
                "one axis plus both aggregate authors"
            );
            let selected_invoke = engine_call(
                &database,
                &[
                    "invoke".into(),
                    run_id.clone(),
                    "intent-review".into(),
                    "--assignments".into(),
                    selected_ids.join(","),
                ],
            );
            assert_eq!(
                selected_invoke["status"], "completed",
                "selected invoke: {selected_invoke}"
            );
            let selected_invocation_id = selected_invoke["result"]["invocation_id"]
                .as_str()
                .expect("selected invocation ID")
                .to_owned();
            let selected_show = (0..120)
                .find_map(|_| {
                    let shown = engine_call(
                        &database,
                        &[
                            "show".into(),
                            "--view".into(),
                            "full".into(),
                            run_id.clone(),
                        ],
                    );
                    let invocation = shown["result"]["work_slot_invocations"]
                        .as_array()
                        .expect("invocations")
                        .iter()
                        .find(|row| row["invocation_id"] == selected_invocation_id)?;
                    match invocation["status"].as_str() {
                        Some("succeeded") | Some("failed") => Some(shown),
                        _ => {
                            thread::sleep(Duration::from_millis(50));
                            None
                        }
                    }
                })
                .expect("selected invocation completion");
            let selected_invocation = selected_show["result"]["work_slot_invocations"]
                .as_array()
                .expect("invocations")
                .iter()
                .find(|row| row["invocation_id"] == selected_invocation_id)
                .expect("selected invocation");
            assert_eq!(
                selected_invocation["status"], "succeeded",
                "selected invocation: {selected_invocation}"
            );
            let selected_workers = selected_invocation["inner_workers"]
                .as_array()
                .expect("selected inner workers");
            assert_eq!(selected_workers.len(), selected_ids.len());
            assert_eq!(
                selected_workers
                    .iter()
                    .map(|worker| worker["assignment_id"].as_str().expect("assignment ID"))
                    .collect::<Vec<_>>(),
                selected_ids.iter().map(String::as_str).collect::<Vec<_>>()
            );
        }
    }
}
