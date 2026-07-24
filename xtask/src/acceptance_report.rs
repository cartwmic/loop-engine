//! Deterministic final acceptance report for reference behaviors, invariants, and facets.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;

use crate::operation_coverage::{self, CoverageMode};

const COVERAGE_PATH: &str = "docs/change/initial-implementation/coverage.md";
const INVARIANTS_PATH: &str = "docs/invariants.md";
const REFERENCE_E2E_PATH: &str = "crates/loop-engine-cli/tests/e2e/reference_acceptance.rs";
const FACET_DIRECTORY: &str = "quality/facets/v1";
const REPORT_SCHEMA_PATH: &str = "quality/evidence/v1/acceptance-report.schema.json";
const EXPECTED_INVARIANTS: usize = 47;
const EXPECTED_REFERENCE_BEHAVIORS: usize = 21;

const REFERENCE_GROUPS: &[(std::ops::RangeInclusive<u8>, &str)] = &[
    (
        1..=4,
        "reference_behaviors_1_through_4_creation_happy_path_and_rejections",
    ),
    (
        5..=9,
        "reference_behaviors_5_through_9_revision_cycles_and_verdict_consistency",
    ),
    (
        10..=13,
        "reference_behaviors_10_through_13_evidence_restart_drift_and_compatibility",
    ),
    (
        14..=17,
        "reference_behaviors_14_through_17_guidance_neutrality_journal_and_interaction",
    ),
    (
        18..=21,
        "reference_behaviors_18_through_21_attempt_resolution_automation_and_visibility",
    ),
];

#[derive(Debug, Serialize)]
struct AcceptanceReport {
    schema_version: u8,
    candidate_revision: String,
    status: &'static str,
    reference_behaviors: Vec<EvidenceRow>,
    invariants: Vec<EvidenceRow>,
    facets: Vec<FacetRow>,
}

#[derive(Debug, Serialize)]
struct EvidenceRow {
    key: String,
    status: &'static str,
    evidence: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FacetRow {
    key: String,
    operation_id: String,
    status: &'static str,
    evidence: Vec<String>,
}

/// Validate final acceptance closure and optionally write its deterministic report.
pub fn run(root: Option<&Path>, revision: &str, output: Option<&Path>) -> Result<()> {
    let root = root.map_or_else(workspace_root, Path::to_path_buf);
    operation_coverage::run_at(&root, CoverageMode::Final, "")?;
    validate_report_schema(&root)?;

    let reference_source = read(&root, REFERENCE_E2E_PATH)?;
    let coverage = read(&root, COVERAGE_PATH)?;
    let invariants_doc = read(&root, INVARIANTS_PATH)?;

    let final_operations = operation_coverage::final_operation_ids_at(&root)?;
    let reference_behaviors = reference_rows(&root, &reference_source, &coverage)?;
    let invariants = invariant_rows(&root, &invariants_doc, &coverage)?;
    let facets = facet_rows(&root, &final_operations)?;

    ensure!(
        reference_behaviors.len() == EXPECTED_REFERENCE_BEHAVIORS,
        "reference acceptance inventory must contain exactly {EXPECTED_REFERENCE_BEHAVIORS} rows"
    );
    ensure!(
        invariants.len() == EXPECTED_INVARIANTS,
        "invariant acceptance inventory must contain exactly {EXPECTED_INVARIANTS} rows"
    );
    ensure!(
        facets.len() == final_operations.len(),
        "facet acceptance inventory must contain exactly one row per final operation"
    );

    let report = AcceptanceReport {
        schema_version: 1,
        candidate_revision: revision.to_owned(),
        status: "passed",
        reference_behaviors,
        invariants,
        facets,
    };

    if let Some(output) = output {
        let output = if output.is_absolute() {
            output.to_path_buf()
        } else {
            root.join(output)
        };
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create report directory {}", parent.display()))?;
        }
        let mut bytes =
            serde_json::to_vec_pretty(&report).context("serialize acceptance report")?;
        bytes.push(b'\n');
        fs::write(&output, bytes)
            .with_context(|| format!("write acceptance report {}", output.display()))?;
    }

    println!(
        "acceptance report passed: {} reference behaviors, {} invariants, {} operation facets",
        EXPECTED_REFERENCE_BEHAVIORS,
        EXPECTED_INVARIANTS,
        final_operations.len()
    );
    Ok(())
}

fn validate_report_schema(root: &Path) -> Result<()> {
    let schema: Value = serde_json::from_str(&read(root, REPORT_SCHEMA_PATH)?)
        .context("parse acceptance report schema")?;
    ensure!(schema["properties"]["schema_version"]["const"] == 1);
    ensure!(schema["properties"]["status"]["const"] == "passed");
    ensure!(
        schema["properties"]["reference_behaviors"]["minItems"] == EXPECTED_REFERENCE_BEHAVIORS
    );
    ensure!(
        schema["properties"]["reference_behaviors"]["maxItems"] == EXPECTED_REFERENCE_BEHAVIORS
    );
    ensure!(schema["properties"]["invariants"]["minItems"] == EXPECTED_INVARIANTS);
    ensure!(schema["properties"]["invariants"]["maxItems"] == EXPECTED_INVARIANTS);
    ensure!(schema["properties"]["facets"]["minItems"] == 21);
    ensure!(schema["properties"]["facets"]["maxItems"] == 21);
    Ok(())
}

fn reference_rows(root: &Path, source: &str, coverage: &str) -> Result<Vec<EvidenceRow>> {
    let mut rows = Vec::new();
    for (range, function) in REFERENCE_GROUPS {
        ensure!(
            source.contains(&format!("fn {function}")),
            "reference acceptance function `{function}` is missing"
        );
        for number in range.clone() {
            ensure!(
                coverage
                    .lines()
                    .any(|line| line.starts_with(&format!("| {number} |"))),
                "reference behavior {number} is missing from {COVERAGE_PATH}"
            );
            let key = format!("reference:{number:02}");
            ensure!(
                coverage.contains(&format!("| {key} |")),
                "reference behavior {number} lacks stable final evidence key `{key}`"
            );
            let evidence = vec![
                format!("{REFERENCE_E2E_PATH}::{function}"),
                "test-support/providers/reference-provider/tests/semantic.rs".to_owned(),
                "quality/facets/v1/run.request.json".to_owned(),
            ];
            ensure_evidence_paths(root, &evidence)?;
            rows.push(EvidenceRow {
                key,
                status: "closed",
                evidence,
            });
        }
    }
    Ok(rows)
}

fn invariant_rows(root: &Path, invariants_doc: &str, coverage: &str) -> Result<Vec<EvidenceRow>> {
    let mut ids = BTreeSet::new();
    for line in invariants_doc.lines() {
        let Some(rest) = line.strip_prefix("### I") else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            ids.insert(digits.parse::<u8>().context("parse invariant number")?);
        }
    }
    ensure!(
        ids.len() == EXPECTED_INVARIANTS,
        "{INVARIANTS_PATH} must declare exactly {EXPECTED_INVARIANTS} invariants"
    );

    let mut rows = Vec::new();
    for number in ids {
        let key = format!("acceptance:I{number}");
        ensure!(
            coverage.contains(&format!("| {key} |")),
            "invariant I{number} lacks stable final evidence key `{key}`"
        );
        let evidence = vec![
            INVARIANTS_PATH.to_owned(),
            COVERAGE_PATH.to_owned(),
            "quality/facets/v1/README.md".to_owned(),
        ];
        ensure_evidence_paths(root, &evidence)?;
        rows.push(EvidenceRow {
            key,
            status: "closed",
            evidence,
        });
    }
    Ok(rows)
}

fn facet_rows(root: &Path, operation_ids: &BTreeSet<String>) -> Result<Vec<FacetRow>> {
    let mut rows = Vec::new();
    for operation_id in operation_ids {
        let path = format!("{FACET_DIRECTORY}/{operation_id}.json");
        let value: Value = serde_json::from_str(&read(root, &path)?)
            .with_context(|| format!("parse facet manifest {path}"))?;
        ensure!(
            value["operation_id"] == operation_id.as_str(),
            "facet manifest operation mismatch in {path}"
        );
        let evidence = vec![path.clone()];
        ensure_evidence_paths(root, &evidence)?;
        rows.push(FacetRow {
            key: format!("facet:{operation_id}"),
            operation_id: operation_id.to_owned(),
            status: "closed",
            evidence,
        });
    }
    Ok(rows)
}

fn ensure_evidence_paths(root: &Path, evidence: &[String]) -> Result<()> {
    for item in evidence {
        let path = item
            .split_once("::")
            .map_or(item.as_str(), |(path, _)| path);
        ensure!(
            root.join(path).is_file(),
            "acceptance evidence path does not resolve: {path}"
        );
    }
    Ok(())
}

fn read(root: &Path, relative: &str) -> Result<String> {
    fs::read_to_string(root.join(relative))
        .with_context(|| format!("read acceptance input {}", root.join(relative).display()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has workspace parent")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_acceptance_inventory_is_closed() {
        run(Some(&workspace_root()), "test-revision", None).expect("acceptance report passes");
    }

    #[test]
    fn reference_groups_cover_exactly_one_through_twenty_one() {
        let numbers: Vec<_> = REFERENCE_GROUPS
            .iter()
            .flat_map(|(range, _)| range.clone())
            .collect();
        assert_eq!(numbers, (1..=21).collect::<Vec<_>>());
    }
}
