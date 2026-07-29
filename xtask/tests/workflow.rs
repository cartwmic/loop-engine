use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn workflow() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows/quality.yml"),
    )
    .unwrap()
}

fn step<'a>(workflow: &'a str, name: &str) -> &'a str {
    let marker = format!("      - name: {name}\n");
    let start = workflow.find(&marker).expect("workflow step") + marker.len();
    let rest = &workflow[start..];
    let end = rest.find("\n      - name: ").unwrap_or(rest.len());
    &rest[..end]
}

fn project(raw: &[u8], fallback: &str) -> (BTreeMap<String, String>, Vec<u8>) {
    let workflow = workflow();
    let projection = step(&workflow, "Project exact CI push event");
    let marker = "          python3 - <<'PY'\n";
    let script = projection
        .split_once(marker)
        .unwrap()
        .1
        .split_once("\n          PY")
        .unwrap()
        .0
        .lines()
        .map(|line| line.strip_prefix("          ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    let root = TempDir::new().unwrap();
    let source = root.path().join("event.json");
    let destination = root.path().join("projection.json");
    let github_output = root.path().join("github-output");
    fs::write(&source, raw).unwrap();
    let output = Command::new("python3")
        .args(["-c", &script])
        .env("SOURCE_EVENT_PATH", &source)
        .env("CI_EVENT_PATH", &destination)
        .env("GITHUB_OUTPUT", &github_output)
        .env("FALLBACK_REVISION", fallback)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let outputs = fs::read_to_string(github_output)
        .unwrap()
        .lines()
        .map(|line| {
            let (key, value) = line.split_once('=').unwrap();
            (key.to_owned(), value.to_owned())
        })
        .collect();
    (outputs, fs::read(destination).unwrap())
}

#[test]
fn final_ci_is_push_only_and_uses_one_publication_lifecycle() {
    let workflow = workflow();
    assert!(workflow.contains("on:\n  push:"));
    for retired in [
        "pull_request",
        "workflow_dispatch",
        "trusted base",
        "trusted-base",
        "branch protection",
        "semantic-judge/v1",
    ] {
        assert!(
            !workflow.contains(retired),
            "retired CI reference: {retired}"
        );
    }
    assert_eq!(
        workflow
            .matches("cargo xtask validate --publication --ci-event")
            .count(),
        1
    );
    assert_eq!(
        workflow
            .matches("uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5")
            .count(),
        3
    );
    let projection_index = workflow
        .find("- name: Project exact CI push event")
        .unwrap();
    let import_index = workflow
        .find("- name: Import prior content revision")
        .unwrap();
    let checkout_index = workflow.find("- name: Checkout pushed repository").unwrap();
    assert!(projection_index < import_index && import_index < checkout_index);

    let import = step(&workflow, "Import prior content revision");
    assert!(import.contains("if: steps.event.outputs.base == 'true'"));
    assert!(import.contains("fetch-depth: 1"));
    assert!(import.contains("ref: ${{ github.event.before }}"));
    assert!(import.contains("persist-credentials: false"));
    assert!(
        !import.contains("path:"),
        "prior object must enter shared repository"
    );

    let checkout = step(&workflow, "Checkout pushed repository");
    assert!(checkout.contains("if: steps.event.outputs.checkout_revision != ''"));
    assert!(checkout.contains("clean: false"));
    assert!(checkout.contains("fetch-depth: 1"));
    assert!(checkout.contains("ref: ${{ steps.event.outputs.checkout_revision }}"));
    assert!(checkout.contains("persist-credentials: false"));
    assert!(
        !checkout.contains("path:"),
        "checkouts must share object database"
    );

    let fallback = step(&workflow, "Checkout fallback repository");
    assert!(fallback.contains("if: steps.event.outputs.checkout_revision == ''"));
    assert!(fallback.contains("ref: ${{ github.event.repository.default_branch }}"));
    assert!(fallback.contains("persist-credentials: false"));
    assert!(!fallback.contains("path:"));
    assert!(workflow.contains("if: always() && steps.validate.outputs.status != ''"));
    assert!(workflow.contains(".git/loop-engine/validation/v1/reports"));
    assert!(workflow.contains(".git/loop-engine/validation/v1/attempts"));
    assert!(workflow.contains("validation.stdout"));
    assert!(workflow.contains("validation.stderr"));
    assert!(workflow.contains("validation.status"));
}

#[test]
fn final_ci_pins_tools_and_keeps_credentials_external() {
    let workflow = workflow();
    for required in [
        "toolchain: \"1.95.0\"",
        "components: rustfmt, clippy",
        "cargo install cargo-deny --locked --version 0.20.2",
        "jdx/mise-action@9e7f7633ff6f6d6048a9418a68d48f288f50eb14",
        "mise install go@1.26.5",
        "MISE_AUTO_INSTALL: \"false\"",
        "MISE_AUTO_INSTALL_DISABLE_TOOLS: go",
        "@mariozechner/pi-coding-agent@0.73.1",
        "sha512-gXQh3SaZmWTfVMc4Ao5+LGbVeKvzyO7tolok0nLsZgq9nGjZx/EEU3NM8C+qUnB4Nvs2rswG5qOVgLzQkq0fHQ==",
        "secrets.LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON",
        "LOOP_ENGINE_SEMANTIC_JUDGE_PROVIDER: openai-codex",
        "LOOP_ENGINE_SEMANTIC_JUDGE_MODEL: gpt-5.6-sol",
    ] {
        assert!(
            workflow.contains(required),
            "missing CI contract: {required}"
        );
    }
    assert!(workflow.contains("if: steps.event.outputs.content == 'true'"));

    for line in workflow.lines().map(str::trim) {
        let Some(action) = line.strip_prefix("uses: ") else {
            continue;
        };
        let revision = action
            .split_once('@')
            .unwrap()
            .1
            .split_whitespace()
            .next()
            .unwrap();
        assert_eq!(
            revision.len(),
            40,
            "action is not pinned by full SHA: {line}"
        );
        assert!(
            revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "action pin is not hexadecimal: {line}"
        );
    }
}

#[test]
fn projection_is_closed_canonical_or_exact_raw_fallback() {
    let workflow = workflow();
    assert!(workflow.contains("source = Path(os.environ[\"SOURCE_EVENT_PATH\"]).read_bytes()"));
    assert!(workflow.contains("destination.write_bytes(source)"));
    assert!(
        workflow
            .contains("projection = {\"before\": before, \"after\": after, \"ref\": reference}")
    );
    assert!(workflow.contains("separators=(\",\", \":\")"));
    assert!(workflow.contains("object_pairs_hook=closed_object"));
    assert!(workflow.contains("base = content and set(before) != {\"0\"}"));
    assert!(workflow.contains("output.write(f\"base={'true' if base else 'false'}\\n\")"));
    assert!(workflow.contains("output.write(f\"checkout_revision={checkout_revision}\\n\")"));
}

#[test]
fn projection_selects_nonzero_checkout_for_new_deletion_and_malformed_events() {
    const ZERO: &str = "0000000000000000000000000000000000000000";
    const BEFORE: &str = "1111111111111111111111111111111111111111";
    const AFTER: &str = "2222222222222222222222222222222222222222";
    const FALLBACK: &str = "3333333333333333333333333333333333333333";

    let new_branch = format!(r#"{{"before":"{ZERO}","after":"{AFTER}","ref":"refs/heads/new"}}"#);
    let (outputs, projected) = project(new_branch.as_bytes(), FALLBACK);
    assert_eq!(outputs["content"], "true");
    assert_eq!(outputs["base"], "false");
    assert_eq!(outputs["checkout_revision"], AFTER);
    assert_eq!(projected, new_branch.as_bytes());

    let deletion = format!(r#"{{"before":"{BEFORE}","after":"{ZERO}","ref":"refs/heads/old"}}"#);
    let (outputs, projected) = project(deletion.as_bytes(), FALLBACK);
    assert_eq!(outputs["content"], "false");
    assert_eq!(outputs["base"], "false");
    assert_eq!(outputs["checkout_revision"], FALLBACK);
    assert_eq!(projected, deletion.as_bytes());
    let (outputs, _) = project(deletion.as_bytes(), ZERO);
    assert_eq!(outputs["checkout_revision"], "");

    let malformed = format!(r#"{{"before":"{ZERO}","after":"{ZERO}","ref":"refs/heads/old"}}"#);
    let (outputs, projected) = project(malformed.as_bytes(), FALLBACK);
    assert_eq!(outputs["content"], "false");
    assert_eq!(outputs["base"], "false");
    assert_eq!(outputs["checkout_revision"], FALLBACK);
    assert_eq!(projected, malformed.as_bytes());

    let (outputs, _) = project(malformed.as_bytes(), ZERO);
    assert_eq!(outputs["checkout_revision"], "");
}
