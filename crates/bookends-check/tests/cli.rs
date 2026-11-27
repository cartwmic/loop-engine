mod common;

use std::process::Command;

use common::{bin, green_graph, isolate, write_prd, Repo};

fn run(repo: &Repo, extra: &[&str]) -> (i32, String, String) {
    let state = tempfile::tempdir().expect("state tempdir");
    let output = isolate(&mut Command::new(bin()), state.path())
        .current_dir(repo.path())
        .args(extra)
        .output()
        .expect("spawn bookends-check");
    (
        output.status.code().unwrap_or(255),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn help_exits_zero() {
    let state = tempfile::tempdir().expect("state tempdir");
    let output = isolate(&mut Command::new(bin()), state.path())
        .arg("--help")
        .output()
        .expect("spawn");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("bookends-check"), "{stdout}");
    assert!(stdout.contains("--repo"), "{stdout}");
    assert!(stdout.contains("--bypass"), "{stdout}");
}

#[test]
fn version_flags_print_name_and_package_version_on_stdout() {
    for flag in ["--version", "-V"] {
        let state = tempfile::tempdir().expect("state tempdir");
        let output = isolate(&mut Command::new(bin()), state.path())
            .arg(flag)
            .output()
            .expect("spawn");
        assert_eq!(output.status.code(), Some(0), "flag={flag}");
        assert!(output.stderr.is_empty(), "flag={flag}: {:?}", output.stderr);
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("bookends-check {}\n", env!("CARGO_PKG_VERSION")),
            "flag={flag}"
        );
    }
}

#[test]
fn usage_unknown_flag_exits_two() {
    let state = tempfile::tempdir().expect("state tempdir");
    let output = isolate(&mut Command::new(bin()), state.path())
        .arg("--nope")
        .output()
        .expect("spawn");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn help_version_and_usage_errors_write_no_action_log_line() {
    for args in [
        vec!["--help"],
        vec!["--version"],
        vec!["-V"],
        vec!["--nope"],
    ] {
        let state = tempfile::tempdir().expect("state tempdir");
        isolate(&mut Command::new(bin()), state.path())
            .args(&args)
            .output()
            .expect("spawn");
        assert!(
            !state.path().join("state/bookends-check").exists(),
            "{args:?} wrote to the state directory"
        );
    }
}

#[test]
fn bypass_flag_on_red_graph_prints_bypass_and_exits_zero() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
         ### LE-2: Uncovered\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("red graph");

    let (code, stdout, _) = run(&repo, &["--bypass", "test:reason"]);
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "BYPASS", "{stdout}");
    assert!(stdout.contains("test"), "{stdout}");
    assert!(stdout.contains("reason"), "{stdout}");
    assert_eq!(code, 0);

    let (code, stdout, _) = run(&repo, &[]);
    let first = stdout.lines().next().unwrap_or("");
    assert_eq!(first, "RED", "{stdout}");
    assert_eq!(code, 1);
}

#[test]
fn green_graph_prints_green_and_exits_zero() {
    let repo = Repo::new();
    green_graph(&repo);
    let (code, stdout, _) = run(&repo, &["--repo", repo.path().to_str().unwrap()]);
    assert_eq!(stdout.lines().next().unwrap_or(""), "GREEN");
    assert_eq!(code, 0);
}

#[test]
fn empty_bypass_is_usage() {
    let repo = Repo::new();
    green_graph(&repo);
    let (code, _, _) = run(&repo, &["--bypass", "test"]);
    assert_eq!(code, 2);
}

#[test]
fn candidate_command_only_parses_the_candidate() {
    let repo = Repo::new();
    repo.write(
        "prd-candidate.md",
        "### LE-7: Candidate\n- Status: live\n- Coverage: e2e/journey\n",
    );
    let (code, stdout, _) = run(&repo, &["candidate", "prd-candidate.md"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.lines().next(), Some("GREEN"));
}

#[test]
fn malformed_candidate_is_red_without_repository_configuration() {
    let repo = Repo::new();
    repo.write("prd-candidate.md", "### LE-7: Missing status\n");
    let (code, stdout, _) = run(&repo, &["validate-candidate", "prd-candidate.md"]);
    assert_eq!(code, 1);
    assert_eq!(stdout.lines().next(), Some("RED"));
    assert!(stdout.contains("Status"), "{stdout}");
}

/// Run the checker with an isolated state directory and return the exit code
/// with the action log lines it wrote.
fn run_logged(repo: &Repo, args: &[&str]) -> (i32, Vec<serde_json::Value>) {
    let state = tempfile::tempdir().expect("state tempdir");
    let output = isolate(&mut Command::new(bin()), state.path())
        .current_dir(repo.path())
        .args(args)
        .output()
        .expect("spawn bookends-check");
    let log = std::fs::read_to_string(state.path().join("state/bookends-check/log.jsonl"))
        .expect("action log in the state directory");
    let records = log
        .lines()
        .map(|line| serde_json::from_str(line).expect("log line is JSON"))
        .collect();
    (output.status.code().unwrap_or(255), records)
}

fn head(repo: &Repo) -> String {
    let output = Command::new("git")
        .current_dir(repo.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("spawn git");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn finding_text(record: &serde_json::Value) -> String {
    record["finding_text"]
        .as_array()
        .expect("finding_text is a list")
        .iter()
        .map(|finding| finding.as_str().expect("finding is text"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn red_check_log_line_names_the_commit_and_the_failing_finding() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
         ### LE-3: Uncovered\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("add an uncovered requirement");

    let (code, records) = run_logged(&repo, &[]);
    assert_eq!(code, 1);
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    assert_eq!(record["outcome"], "red");
    assert_eq!(record["commit"], head(&repo).as_str(), "{record}");
    assert_eq!(
        record["findings"],
        record["finding_text"].as_array().unwrap().len()
    );
    assert!(finding_text(record).contains("LE-3"), "{record}");
}

#[test]
fn red_candidate_log_line_names_the_file_and_the_failing_finding() {
    let repo = Repo::new();
    repo.write("cand-bad.md", "### LE-7: Missing status\n");

    let (code, records) = run_logged(&repo, &["candidate", "cand-bad.md"]);
    assert_eq!(code, 1);
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    assert_eq!(record["action"], "candidate");
    assert_eq!(record["outcome"], "red");
    assert_eq!(record["candidate"], "cand-bad.md", "{record}");
    assert!(finding_text(record).contains("LE-7"), "{record}");
}

#[test]
fn green_runs_keep_their_line_and_carry_no_finding_text() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write(
        "cand-ok.md",
        "### LE-7: Candidate\n- Status: live\n- Coverage: e2e/journey\n",
    );

    let (code, records) = run_logged(&repo, &[]);
    assert_eq!(code, 0);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["outcome"], "green");
    assert_eq!(records[0]["commit"], head(&repo).as_str(), "{}", records[0]);
    assert!(finding_text(&records[0]).is_empty(), "{}", records[0]);

    let (code, records) = run_logged(&repo, &["candidate", "cand-ok.md"]);
    assert_eq!(code, 0);
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["outcome"], "green");
    assert_eq!(records[0]["candidate"], "cand-ok.md");
    assert!(finding_text(&records[0]).is_empty(), "{}", records[0]);
}

#[test]
fn candidate_that_cannot_be_read_names_the_file_in_its_log_line() {
    let repo = Repo::new();
    let (code, records) = run_logged(&repo, &["candidate", "missing.md"]);
    assert_eq!(code, 1);
    assert_eq!(records[0]["candidate"], "missing.md", "{}", records[0]);
    assert!(records[0]["error"].is_string(), "{}", records[0]);
}
