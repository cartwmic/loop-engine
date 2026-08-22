mod common;

use bookends_check::CheckStatus;
use common::{green_graph, write_prd, Repo, GREEN_PRD, GREEN_TOML};

fn assert_green(repo: &Repo) {
    let report = repo.check();
    assert_eq!(
        report.status,
        CheckStatus::Green,
        "expected GREEN, findings={:?}",
        report.findings
    );
    assert!(report.findings.is_empty());
}

fn assert_red(repo: &Repo) -> Vec<String> {
    let report = repo.check();
    assert_eq!(
        report.status,
        CheckStatus::Red,
        "expected RED, status={:?}",
        report.status
    );
    assert!(
        !report.findings.is_empty(),
        "RED must have non-empty findings"
    );
    report.findings
}

#[test]
fn green_python_citation() {
    let repo = Repo::new();
    green_graph(&repo);
    assert_green(&repo);
    let report = repo.check();
    assert_eq!(report.live_ids, vec!["LE-1".to_string()]);
}

#[test]
fn bypass_on_green_stays_green() {
    let repo = Repo::new();
    green_graph(&repo);
    let report = repo.check_bypass("test", "reason");
    assert_eq!(report.status, CheckStatus::Green);
    assert!(report.findings.is_empty());
}

#[test]
fn mixed_allowlisted_and_unparsed_commands_stay_green() {
    let repo = Repo::new();
    repo.write("bookends.toml", GREEN_TOML);
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  journey:
    runs-on: ubuntu-latest
    steps:
      - run: bash -lc "echo setup"
      - run: python3 tests/journey.py
      - run: cargo clippy --workspace
"#,
    );
    repo.write("tests/journey.py", "# bookends:LE-1\nprint('ok')\n");
    repo.commit_all("mixed runner");
    assert_green(&repo);
}

#[test]
fn bypass_converts_red_and_clears_findings() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
         ### LE-2: Uncovered\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("add uncovered id");
    assert_red(&repo);
    let bypassed = repo.check_bypass("test", "reason");
    assert_eq!(
        bypassed.status,
        CheckStatus::Bypass {
            class: "test".into(),
            reason: "reason".into(),
        }
    );
    assert!(bypassed.findings.is_empty());
}

#[test]
fn dangling_tag_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write("tests/journey.py", "# bookends:LE-99\nprint('ok')\n");
    repo.commit_all("dangling");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("dangling") || f.contains("LE-99")),
        "{findings:?}"
    );
}

#[test]
fn noncanonical_at_spec_tag_is_red_even_with_valid_coverage() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write(
        "tests/journey.py",
        "# @spec:UPL-1\n# bookends:LE-1\nprint('ok')\n",
    );
    repo.commit_all("noncanonical citation");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("non-canonical")),
        "{findings:?}"
    );
}

#[test]
fn tombstoned_tag_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: tombstone\n\n\
         ### LE-2: Still live\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write(
        "tests/journey.py",
        "# bookends:LE-1\n# bookends:LE-2\nprint('ok')\n",
    );
    repo.commit_all("tombstone LE-1");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("tombstoned")),
        "{findings:?}"
    );
}

#[test]
fn missing_mandatory_e2e_citation_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write("tests/journey.py", "print('untagged')\n");
    repo.commit_all("drop citation");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("no eligible") && f.contains("e2e/journey")),
        "{findings:?}"
    );
}

#[test]
fn undeclared_contract_on_live_record_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey, contract\n",
    );
    repo.commit_all("undeclared contract");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("undeclared")),
        "{findings:?}"
    );
}

#[test]
fn declared_contract_without_citation_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]

[classes.contract]
pathspecs = ["contracts/**"]
required_ci_jobs = ["journey"]
"#,
    );
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey, contract\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  journey:
    runs-on: ubuntu-latest
    steps:
      - run: python3 tests/journey.py
      - run: python3 contracts/api.py
"#,
    );
    repo.write("tests/journey.py", "# bookends:LE-1\nprint('ok')\n");
    repo.write("contracts/api.py", "print('no tag')\n");
    repo.commit_all("contract undeclared citation");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("contract")),
        "{findings:?}"
    );
}

#[test]
fn declared_contract_with_citation_is_green() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]

[classes.contract]
pathspecs = ["contracts/**"]
required_ci_jobs = ["journey"]
"#,
    );
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey, contract\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  journey:
    runs-on: ubuntu-latest
    steps:
      - run: python3 tests/journey.py
      - run: python3 contracts/api.py
"#,
    );
    repo.write("tests/journey.py", "# bookends:LE-1\nprint('ok')\n");
    repo.write("contracts/api.py", "# bookends:LE-1\nprint('ok')\n");
    repo.commit_all("contract covered");
    assert_green(&repo);
}

#[test]
fn malformed_prd_is_red_not_err() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(&repo, "### LE-1: Missing status\njust prose\n");
    repo.commit_all("malformed");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("Status") || f.contains("malformed")),
        "{findings:?}"
    );
}

#[test]
fn missing_bookends_toml_is_red() {
    let repo = Repo::new();
    repo.write("README.md", "no bookends\n");
    repo.commit_all("empty");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("bookends.toml")),
        "{findings:?}"
    );
}

#[test]
fn first_enable_is_green_without_ancestor() {
    let repo = Repo::new();
    green_graph(&repo);
    assert_green(&repo);
}

#[test]
fn empty_enabled_prd_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(&repo, "### 3.1 Goals\n\nNo requirement records.\n");
    repo.commit_all("empty prd");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("nothing-to-validate") || f.contains("no live or tombstone")),
        "{findings:?}"
    );
}

#[test]
fn title_change_under_same_live_id_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Renamed requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
         The statement evolved.\n",
    );
    repo.commit_all("reassign title");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("changed title")),
        "{findings:?}"
    );
}

#[test]
fn live_id_matching_parent_tombstone_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: tombstone\n\n\
         ### LE-2: Replacement\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write("tests/journey.py", "# bookends:LE-2\nprint('ok')\n");
    repo.commit_all("retire LE-1");
    assert_green(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
         ### LE-2: Replacement\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write(
        "tests/journey.py",
        "# bookends:LE-1\n# bookends:LE-2\nprint('ok')\n",
    );
    repo.commit_all("reuse LE-1");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("revived") && f.contains("LE-1")),
        "{findings:?}"
    );
}

#[test]
fn malformed_immediate_parent_is_not_skipped() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(&repo, "### LE-1: broken\n");
    repo.commit_all("malformed intermediate");
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write("tests/journey.py", "# bookends:LE-1\nprint('ok')\n");
    repo.commit_all("restore");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|finding| finding.contains("malformed") && finding.contains("Status")),
        "{findings:?}"
    );
    write_prd(
        &repo,
        "### LE-2: Other\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write("tests/journey.py", "# bookends:LE-2\nprint('ok')\n");
    repo.commit_all("drop LE-1 without tombstone");
    // The immediate malformed parent, rather than an older valid commit,
    // remains the only continuity input.
}

#[test]
fn empty_enabled_parent_has_no_requirement_identity_to_preserve() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(&repo, "### 3.1 Goals\n\nNo requirement records.\n");
    repo.commit_all("empty intermediate");
    write_prd(
        &repo,
        "### LE-2: Other\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write("tests/journey.py", "# bookends:LE-2\nprint('ok')\n");
    repo.commit_all("new id after empty parent");
    assert_green(&repo);
}

#[test]
fn skip_marker_makes_citation_ineligible() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write(
        "tests/journey.py",
        "# bookends:skip\n# bookends:LE-1\nprint('ok')\n",
    );
    repo.commit_all("skip");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")),
        "{findings:?}"
    );
}

#[test]
fn unparsed_runner_is_red() {
    let repo = Repo::new();
    repo.write("bookends.toml", GREEN_TOML);
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  journey:
    runs-on: ubuntu-latest
    steps:
      - run: bash -lc "python3 tests/journey.py"
"#,
    );
    repo.write("tests/journey.py", "# bookends:LE-1\nprint('ok')\n");
    repo.commit_all("unparsed runner");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("allowlisted") || f.contains("no eligible")),
        "{findings:?}"
    );
}

#[test]
fn removed_tombstone_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: tombstone\n\n\
         ### LE-2: Replacement\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.write("tests/journey.py", "# bookends:LE-2\nprint('ok')\n");
    repo.commit_all("retire LE-1");
    assert_green(&repo);
    write_prd(
        &repo,
        "### LE-2: Replacement\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("drop tombstone LE-1");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("LE-1") && (f.contains("tombstone") || f.contains("absent"))),
        "{findings:?}"
    );
}

#[test]
fn untagged_durable_file_is_advisory_and_still_green() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write("tests/other_journey.py", "print('untagged durable')\n");
    repo.commit_all("untagged extra file");
    assert_green(&repo);
}

#[test]
fn empty_discovery_surface_is_red() {
    let repo = Repo::new();
    repo.write("bookends.toml", GREEN_TOML);
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(".github/workflows/ci.yml", common::GREEN_WORKFLOW);
    repo.write("README.md", "no tests yet\n");
    repo.commit_all("missing e2e surface");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("discovery surface") || f.contains("no tracked")),
        "{findings:?}"
    );
}

#[test]
fn missing_named_job_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  other:
    runs-on: ubuntu-latest
    steps:
      - run: python3 tests/journey.py
"#,
    );
    repo.commit_all("rename job");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("not found")),
        "{findings:?}"
    );
}

#[test]
fn internal_crate_unit_test_comment_is_not_public_journey_coverage() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crates/**/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crates/example"]
resolver = "2"
"#,
    );
    repo.write(
        "crates/example/Cargo.toml",
        r#"[package]
name = "example"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crates/example/src/lib.rs",
        "#[cfg(test)]\nmod tests {\n    // bookends:LE-1\n    #[test]\n    fn internal_only() {}\n}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace
"#,
    );
    repo.commit_all("internal unit test citation");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|finding| { finding.contains("LE-1") && finding.contains("no eligible") }),
        "internal source citation must not satisfy e2e/journey: {findings:?}"
    );
}

#[test]
fn cargo_workspace_collection_is_green() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crate_a/src/lib.rs",
        "// bookends:LE-1\n#[test]\nfn proves_le1() {}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace
"#,
    );
    repo.commit_all("rust workspace");
    assert_green(&repo);
}

#[test]
fn cargo_skipped_integration_test_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/tests/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"

[[test]]
name = "journey"
test = false
"#,
    );
    repo.write("crate_a/tests/journey.rs", "// bookends:LE-1\n");
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace
"#,
    );
    repo.commit_all("skipped integration test");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")),
        "{findings:?}"
    );
}

#[test]
fn cargo_workspace_no_run_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crate_a/src/lib.rs",
        "// bookends:LE-1\n#[test]\nfn proves_le1() {}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace --no-run
"#,
    );
    repo.commit_all("rust workspace no-run");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")
            || f.contains("allowlisted")
            || f.contains("unparsed")),
        "{findings:?}"
    );
}

#[test]
fn cargo_workspace_manifest_path_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crate_a/src/lib.rs",
        "// bookends:LE-1\n#[test]\nfn proves_le1(){}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace --manifest-path nested/Cargo.toml
"#,
    );
    repo.commit_all("rust workspace manifest-path");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")
            || f.contains("allowlisted")
            || f.contains("unparsed")),
        "{findings:?}"
    );
}

#[test]
fn cargo_workspace_bins_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crate_a/src/lib.rs",
        "// bookends:LE-1\n#[test]\nfn proves_le1(){}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace --bins
"#,
    );
    repo.commit_all("rust workspace bins filter");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")
            || f.contains("allowlisted")
            || f.contains("unparsed")),
        "{findings:?}"
    );
}

#[test]
fn cargo_workspace_ignore_rust_version_bins_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crate_a/src/lib.rs",
        "// bookends:LE-1\n#[test]\nfn proves_le1(){}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace --ignore-rust-version --bins
"#,
    );
    repo.commit_all("boolean flag must not swallow bins filter");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")
            || f.contains("allowlisted")
            || f.contains("unparsed")),
        "{findings:?}"
    );
}

#[test]
fn gha_nested_working_directory_is_red() {
    let repo = Repo::new();
    repo.write(
        "bookends.toml",
        r#"prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["crate_a/src/**"]
required_ci_jobs = ["test"]
"#,
    );
    repo.write("docs/PRD.md", GREEN_PRD);
    repo.write(
        "Cargo.toml",
        r#"[workspace]
members = ["crate_a"]
resolver = "2"
"#,
    );
    repo.write(
        "crate_a/Cargo.toml",
        r#"[package]
name = "crate_a"
version = "0.1.0"
edition = "2021"
"#,
    );
    repo.write(
        "crate_a/src/lib.rs",
        "// bookends:LE-1\n#[test]\nfn proves_le1(){}\n",
    );
    repo.write(
        ".github/workflows/ci.yml",
        r#"name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: cargo test --workspace
        working-directory: crate_a
"#,
    );
    repo.commit_all("nested working-directory is not root collection");
    let findings = assert_red(&repo);
    assert!(
        findings.iter().any(|f| f.contains("no eligible")
            || f.contains("allowlisted")
            || f.contains("unparsed")),
        "{findings:?}"
    );
}

#[test]
fn immediate_red_parent_still_supplies_id_continuity() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n\n\
         ### LE-2: Transient uncovered\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("bypass-landed uncovered LE-2");
    assert_red(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("drop transient LE-2");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("LE-2") && f.contains("disappeared")),
        "{findings:?}"
    );
}

#[test]
fn inaccessible_repo_root_is_err() {
    let err =
        bookends_check::check_repo(std::path::Path::new("/this/does/not/exist-bookends"), None)
            .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn unknown_coverage_class_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Coverage: e2e/journey, go\n",
    );
    repo.commit_all("unknown class");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("unknown coverage") || f.contains("invalid Coverage")),
        "{findings:?}"
    );
}

#[test]
fn duplicate_status_is_red() {
    let repo = Repo::new();
    green_graph(&repo);
    write_prd(
        &repo,
        "### LE-1: Example requirement\n- Status: live\n- Status: live\n- Coverage: e2e/journey\n",
    );
    repo.commit_all("duplicate status");
    let findings = assert_red(&repo);
    assert!(
        findings
            .iter()
            .any(|f| f.contains("duplicate") || f.contains("Status")),
        "{findings:?}"
    );
}
