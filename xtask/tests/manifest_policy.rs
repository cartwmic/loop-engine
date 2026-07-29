use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use xtask::config::{Phase, Scope, SemanticRequirement, load_manifest};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

#[test]
fn final_manifest_is_exact_project_policy_registry() {
    let document = load_manifest(
        &repository_root().join("quality/manifest.toml"),
        SemanticRequirement::Required,
    )
    .expect("load final project manifest");
    let manifest = document.manifest();

    assert_eq!(manifest.schema_version(), 2);
    assert_eq!(manifest.defaults().timeout_seconds(), 900);
    assert_eq!(manifest.defaults().max_output_bytes(), 8_388_608);
    assert_eq!(
        manifest.defaults().environment().unset(),
        ["RUSTUP_TOOLCHAIN"]
    );
    assert_eq!(
        manifest.defaults().environment().set(),
        &BTreeMap::from([
            ("CARGO_TARGET_DIR".to_owned(), "{target_root}".to_owned()),
            ("GOCACHE".to_owned(), "{cache_root}".to_owned()),
            ("MISE_AUTO_INSTALL".to_owned(), "false".to_owned()),
            (
                "MISE_AUTO_INSTALL_DISABLE_TOOLS".to_owned(),
                "go".to_owned(),
            ),
            ("TMPDIR".to_owned(), "{scratch_root}".to_owned()),
        ])
    );

    let expected_inputs = [
        ".cargo",
        ".githooks",
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "rustfmt.toml",
        "deny.toml",
        "xtask",
        "quality",
    ];
    assert_eq!(
        manifest.runner().inputs(),
        expected_inputs.map(PathBuf::from).as_slice()
    );

    let prerequisites = manifest.prerequisites();
    assert_eq!(prerequisites.len(), 2);
    assert_eq!(prerequisites[0].id(), "cargo-deny");
    assert_eq!(prerequisites[0].program(), "cargo");
    assert_eq!(prerequisites[0].args(), ["deny", "--version"]);
    assert_eq!(prerequisites[0].stdout_equals(), Some("cargo-deny 0.20.2"));
    assert_eq!(
        prerequisites[0].install_hint(),
        "cargo install cargo-deny --locked --version 0.20.2"
    );
    assert_eq!(prerequisites[1].id(), "go-1.26.5");
    assert_eq!(prerequisites[1].program(), "mise");
    assert_eq!(prerequisites[1].args(), ["where", "go@1.26.5"]);
    assert_eq!(prerequisites[1].stdout_equals(), None);
    assert_eq!(prerequisites[1].install_hint(), "mise install go@1.26.5");

    let expected_checks: [(&str, &str, &[&str], Scope, &str); 10] = [
        (
            "diff-check",
            "/usr/bin/git",
            &[
                "--git-dir={git_directory}",
                "--literal-pathspecs",
                "diff",
                "--check",
                "{base_revision}",
                "{candidate_revision}",
                "--",
            ],
            Scope::ChangedFiles,
            "{candidate_root}",
        ),
        (
            "fmt",
            "cargo",
            &["fmt", "--all", "--check"],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "check",
            "cargo",
            &["check", "--workspace", "--locked"],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "clippy",
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "test",
            "cargo",
            &["test", "--workspace", "--locked"],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "doc",
            "cargo",
            &["doc", "--workspace", "--no-deps", "--locked"],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "reference-provider-test",
            "cargo",
            &[
                "test",
                "--manifest-path",
                "test-support/providers/reference-provider/Cargo.toml",
                "--locked",
            ],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "scenario-provider-test",
            "cargo",
            &[
                "test",
                "--manifest-path",
                "test-support/providers/scenario-provider/Cargo.toml",
                "--locked",
            ],
            Scope::Repository,
            "{candidate_root}",
        ),
        (
            "reference-go-test",
            "mise",
            &["exec", "go@1.26.5", "--", "go", "test", "./..."],
            Scope::Repository,
            "{candidate_root}/examples/providers/reference-go",
        ),
        (
            "deny",
            "cargo",
            &["deny", "check"],
            Scope::Repository,
            "{candidate_root}",
        ),
    ];

    assert_eq!(manifest.checks().len(), expected_checks.len());
    for (check, (id, program, args, scope, cwd)) in manifest.checks().iter().zip(expected_checks) {
        assert_eq!(check.id(), id);
        assert_eq!(check.phases(), [Phase::PreCommit, Phase::Publication]);
        assert_eq!(check.scope(), scope);
        assert_eq!(check.program(), program);
        assert_eq!(check.args(), args);
        assert_eq!(check.cwd(), cwd);
        assert_eq!(check.timeout_seconds(), 900);
        assert_eq!(check.max_output_bytes(), 8_388_608);
        assert!(check.environment().unset().is_empty());
        assert!(check.environment().set().is_empty());
    }

    let semantic = manifest.semantic().expect("final semantic registry");
    assert_eq!(
        semantic.program(),
        "{candidate_root}/quality/semantic-judge/v2/judge"
    );
    assert!(semantic.args().is_empty());
    assert_eq!(semantic.cwd(), "{candidate_root}");
    assert_eq!(semantic.timeout_seconds(), 900);
    assert_eq!(semantic.max_output_bytes(), 8_388_608);
    assert_eq!(
        semantic.response_schema(),
        Path::new("quality/semantic-judge/v2/response.schema.json")
    );
    assert!(semantic.environment().unset().is_empty());
    assert_eq!(
        semantic.environment().set(),
        &BTreeMap::from([("TMPDIR".to_owned(), "{scratch_root}".to_owned())])
    );
    assert_eq!(
        semantic
            .axes()
            .iter()
            .map(|axis| (axis.id(), axis.rubric()))
            .collect::<Vec<_>>(),
        vec![
            (
                "documentation",
                Path::new("quality/rubrics/documentation.md")
            ),
            (
                "observability",
                Path::new("quality/rubrics/observability.md")
            ),
            ("architecture", Path::new("quality/rubrics/architecture.md")),
            (
                "behavioral-evidence",
                Path::new("quality/rubrics/behavioral-evidence.md"),
            ),
        ]
    );
    assert_eq!(semantic.coherence().id(), "coherence");
    assert_eq!(
        semantic.coherence().rubric(),
        Path::new("quality/rubrics/coherence.md")
    );
}

#[test]
fn final_diff_check_treats_pathspec_magic_filenames_as_literals() {
    let repo = TempDir::new().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(repo.path(), &["config", "user.email", "validation@test"]);
    git(repo.path(), &["config", "user.name", "Validation Test"]);
    fs::write(repo.path().join("seed"), "seed\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "base"]);
    let base = git(repo.path(), &["rev-parse", "HEAD"]);

    let magic_path = ":(exclude)*";
    fs::write(repo.path().join(magic_path), "trailing whitespace \n").unwrap();
    git(
        repo.path(),
        &["--literal-pathspecs", "add", "--", magic_path],
    );
    git(repo.path(), &["commit", "-q", "-m", "candidate"]);
    let candidate = git(repo.path(), &["rev-parse", "HEAD"]);

    let document = load_manifest(
        &repository_root().join("quality/manifest.toml"),
        SemanticRequirement::Required,
    )
    .unwrap();
    let check = &document.manifest().checks()[0];
    assert_eq!(check.id(), "diff-check");
    let git_directory = repo.path().join(".git");
    let mut args: Vec<String> = check
        .args()
        .iter()
        .map(|arg| {
            arg.replace("{git_directory}", &git_directory.to_string_lossy())
                .replace("{base_revision}", &base)
                .replace("{candidate_revision}", &candidate)
        })
        .collect();
    args.push(magic_path.to_owned());
    let output = Command::new(check.program())
        .args(&args)
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(":(exclude)*:1: trailing whitespace"));
}
