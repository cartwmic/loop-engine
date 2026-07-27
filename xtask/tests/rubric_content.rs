use std::fs;
use std::path::{Path, PathBuf};

const SEMANTIC_RUBRIC_FILES: [&str; 5] = [
    "documentation.md",
    "observability.md",
    "architecture.md",
    "behavioral-evidence.md",
    "coherence.md",
];

fn rubrics_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../quality/rubrics")
}

fn rubric(file_name: &str) -> String {
    fs::read_to_string(rubrics_root().join(file_name))
        .unwrap_or_else(|error| panic!("read final rubric `{file_name}`: {error}"))
}

fn assert_contains_all(owner: &str, content: &str, required: &[&str]) {
    for text in required {
        assert!(
            content.contains(text),
            "{owner} rubric must contain final contract text `{text}`"
        );
    }
}

#[test]
fn final_semantic_rubric_inventory_is_exact() {
    for file_name in SEMANTIC_RUBRIC_FILES {
        assert!(
            rubrics_root().join(file_name).is_file(),
            "missing final rubric `{file_name}`"
        );
    }
}

#[test]
fn documentation_rubric_owns_impact_and_consistency_judgment() {
    let content = rubric("documentation.md");
    assert_contains_all(
        "documentation",
        &content,
        &[
            "documentation impact",
            "resulting candidate tree",
            "behavior, architecture, contracts, testing policy, and development policy",
            "Deterministic documentation checks",
        ],
    );
}

#[test]
fn observability_rubric_owns_diagnostic_consequences() {
    let content = rubric("observability.md");
    assert_contains_all(
        "observability",
        &content,
        &[
            "observability consequences",
            "diagnostic, not mutation authority",
            "dispatch, provider-execution, and persistence boundaries",
            "must not overclaim completeness",
        ],
    );
}

#[test]
fn architecture_rubric_owns_internal_direction_and_placement_judgment() {
    let content = rubric("architecture.md");
    assert_contains_all(
        "architecture",
        &content,
        &[
            "model must not depend on capabilities or operations",
            "capabilities must not depend on operations",
            "provider-process and persistence construction",
            "composition.rs",
            "dispatch.rs",
            "raw integration details",
            "KISS",
        ],
    );
    assert!(
        !content.contains("three inward-pointing crates"),
        "objective product-crate direction belongs to workspace metadata tests"
    );
}

#[test]
fn behavioral_evidence_rubric_owns_sufficiency_judgment() {
    let content = rubric("behavioral-evidence.md");
    assert_contains_all(
        "behavioral-evidence",
        &content,
        &[
            "behavioral-evidence sufficiency",
            "black-box production CLI",
            "real provider-process and SQLite integrations",
            "Lower-level schema, protocol, unit, integration, and property tests",
            "deterministic evidence",
        ],
    );
}

#[test]
fn coherence_rubric_preserves_axis_results_and_judges_cross_axis_conflicts() {
    let content = rubric("coherence.md");
    assert_contains_all(
        "coherence",
        &content,
        &[
            "documentation, observability, architecture, and behavioral-evidence",
            "must not upgrade, erase, or weaken",
            "cross-axis",
            "same candidate binding",
        ],
    );
}
