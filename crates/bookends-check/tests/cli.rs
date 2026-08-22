mod common;

use std::process::Command;

use common::{bin, green_graph, write_prd, Repo};

fn run(repo: &Repo, extra: &[&str]) -> (i32, String, String) {
    let output = Command::new(bin())
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
    let output = Command::new(bin()).arg("--help").output().expect("spawn");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("bookends-check"), "{stdout}");
    assert!(stdout.contains("--repo"), "{stdout}");
    assert!(stdout.contains("--bypass"), "{stdout}");
}

#[test]
fn usage_unknown_flag_exits_two() {
    let output = Command::new(bin()).arg("--nope").output().expect("spawn");
    assert_eq!(output.status.code(), Some(2));
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
