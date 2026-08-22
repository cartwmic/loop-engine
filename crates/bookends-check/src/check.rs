use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use crate::config::{parse_repo_config, ClassConfig, RepoConfig};
use crate::continuity::{continuity_findings, preceding_committed_prd};
use crate::eligibility::{
    collection_contains, index_class_files, load_workflow_jobs, workspace_packages,
    IndexedCitation, JobCommands,
};
use crate::git;
use crate::prd::{parse_prd, CoverageClass, Prd, RecordKind};
use crate::{io_err_reading_root, CheckReport};

/// Evaluate the enabled bookends graph at `repo_root`.
///
/// Missing or malformed enabled inputs (including a PRD that parses to zero
/// live or tombstone records) are `CheckStatus::Red`, not `Err`. `Err` is
/// reserved for I/O that cannot read `repo_root`. Bypass converts Red to
/// Bypass and clears findings; it never converts Red into Green.
pub fn check_repo(
    repo_root: &Path,
    bypass: Option<(&str, &str)>,
) -> Result<CheckReport, io::Error> {
    let meta = fs::metadata(repo_root).map_err(|err| io_err_reading_root(repo_root, err))?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("repo root is not a directory: {}", repo_root.display()),
        ));
    }

    let report = evaluate(repo_root);
    Ok(report.apply_bypass(bypass))
}

fn evaluate(repo_root: &Path) -> CheckReport {
    let cfg_path = repo_root.join("bookends.toml");
    let cfg_text = match fs::read_to_string(&cfg_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return CheckReport::red(
                Vec::new(),
                vec!["bookends.toml is missing; enabled inputs fail closed".into()],
            );
        }
        Err(err) => {
            return CheckReport::red(
                Vec::new(),
                vec![format!("cannot read bookends.toml: {err}")],
            );
        }
    };
    let cfg = match parse_repo_config(&cfg_text) {
        Ok(cfg) => cfg,
        Err(err) => return CheckReport::red(Vec::new(), vec![err]),
    };

    let prd_path = git::join_repo(repo_root, &cfg.prd);
    let prd_text = match fs::read_to_string(&prd_path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return CheckReport::red(
                Vec::new(),
                vec![format!(
                    "{} is missing; enabled inputs fail closed",
                    cfg.prd
                )],
            );
        }
        Err(err) => {
            return CheckReport::red(Vec::new(), vec![format!("cannot read {}: {err}", cfg.prd)]);
        }
    };
    let prd = match parse_prd(&prd_text) {
        Ok(prd) => prd,
        Err(errors) => return CheckReport::red(Vec::new(), errors),
    };
    let live_ids = prd.live_ids();

    if !git::is_git_repo(repo_root) {
        let mut findings = prefix_findings(&prd, &cfg);
        findings.push("repository is not a git work tree".into());
        return CheckReport::red(live_ids, findings);
    }

    let mut findings = tree_graph_findings(repo_root, &cfg, &prd);

    match preceding_committed_prd(repo_root, &cfg, &prd_text) {
        Ok(previous) => findings.extend(continuity_findings(&prd, previous.as_ref())),
        Err(err) => findings.push(err),
    }

    if findings.is_empty() {
        CheckReport::green(live_ids)
    } else {
        CheckReport::red(live_ids, findings)
    }
}

fn prefix_findings(prd: &Prd, cfg: &RepoConfig) -> Vec<String> {
    let mut findings = Vec::new();
    if prd.records.is_empty() {
        findings.push(
            "enabled PRD contains no live or tombstone records; nothing-to-validate is allowed only when bookends are off"
                .into(),
        );
    }
    findings.extend(contract_declaration_findings(prd, cfg));
    findings
}

fn tree_graph_findings(repo_root: &Path, cfg: &RepoConfig, prd: &Prd) -> Vec<String> {
    let mut findings = prefix_findings(prd, cfg);

    let tracked = match git::tracked_files(repo_root) {
        Ok(files) => files.into_iter().collect::<BTreeSet<_>>(),
        Err(err) => {
            findings.push(err);
            return findings;
        }
    };

    let jobs = match load_workflow_jobs(repo_root) {
        Ok(jobs) => jobs,
        Err(err) => {
            findings.push(err);
            return findings;
        }
    };
    let packages = match workspace_packages(repo_root) {
        Ok(packages) => packages,
        Err(err) => {
            findings.push(err);
            return findings;
        }
    };

    findings.extend(class_surface_findings(
        repo_root,
        &cfg.e2e_journey,
        "e2e/journey",
        &jobs,
        &tracked,
    ));
    if let Some(contract) = &cfg.contract {
        findings.extend(class_surface_findings(
            repo_root, contract, "contract", &jobs, &tracked,
        ));
    }

    let e2e_files = match git::pathspec_files(repo_root, &cfg.e2e_journey.pathspecs) {
        Ok(files) => files,
        Err(err) => {
            findings.push(err);
            Vec::new()
        }
    };
    let contract_files = match &cfg.contract {
        Some(contract) => match git::pathspec_files(repo_root, &contract.pathspecs) {
            Ok(files) => files,
            Err(err) => {
                findings.push(err);
                Vec::new()
            }
        },
        None => Vec::new(),
    };

    let e2e_citations = match index_class_files(repo_root, &e2e_files) {
        Ok((citations, _)) => citations,
        Err(errors) => {
            findings.extend(errors);
            Vec::new()
        }
    };
    let contract_citations = match index_class_files(repo_root, &contract_files) {
        Ok((citations, _)) => citations,
        Err(errors) => {
            findings.extend(errors);
            Vec::new()
        }
    };

    let live_set = prd.live_id_set();
    let tomb_set = prd.tombstone_ids();
    findings.extend(dangling_findings(&e2e_citations, &live_set, &tomb_set));
    findings.extend(dangling_findings(&contract_citations, &live_set, &tomb_set));

    let e2e_eligible = eligible_ids(
        &e2e_citations,
        &cfg.e2e_journey,
        &jobs,
        &packages,
        &tracked,
        true,
    );
    let contract_eligible = match &cfg.contract {
        Some(contract) => eligible_ids(
            &contract_citations,
            contract,
            &jobs,
            &packages,
            &tracked,
            false,
        ),
        None => BTreeSet::new(),
    };

    for record in &prd.records {
        let RecordKind::Live { classes } = &record.kind else {
            continue;
        };
        for class in classes {
            match class {
                CoverageClass::E2eJourney => {
                    if !e2e_eligible.contains(&record.id) {
                        findings.push(format!(
                            "{} has no eligible {} citation",
                            record.id,
                            class.token()
                        ));
                    }
                }
                CoverageClass::Contract => {
                    if cfg.contract.is_none() {
                        continue;
                    }
                    if !contract_eligible.contains(&record.id) {
                        findings.push(format!(
                            "{} has no eligible {} citation",
                            record.id,
                            class.token()
                        ));
                    }
                }
            }
        }
    }

    findings
}

fn contract_declaration_findings(prd: &Prd, cfg: &RepoConfig) -> Vec<String> {
    let mut findings = Vec::new();
    if cfg.contract.is_some() {
        return findings;
    }
    for record in &prd.records {
        if let RecordKind::Live { classes } = &record.kind {
            if classes.contains(&CoverageClass::Contract) {
                findings.push(format!(
                    "{} declares contract coverage but [classes.contract] is undeclared",
                    record.id
                ));
            }
        }
    }
    findings
}

fn class_surface_findings(
    repo: &Path,
    class: &ClassConfig,
    class_name: &str,
    jobs: &std::collections::BTreeMap<String, JobCommands>,
    tracked: &BTreeSet<String>,
) -> Vec<String> {
    let mut findings = Vec::new();
    match git::pathspec_files(repo, &class.pathspecs) {
        Ok(files) => {
            let tracked_hits: Vec<_> = files.into_iter().filter(|f| tracked.contains(f)).collect();
            if tracked_hits.is_empty() {
                findings.push(format!(
                    "{class_name} discovery surface resolved to no tracked files"
                ));
            }
        }
        Err(err) => findings.push(err),
    }
    for job_id in &class.required_ci_jobs {
        match jobs.get(job_id) {
            None => findings.push(format!(
                "required CI job '{job_id}' was not found in .github/workflows/*.yml"
            )),
            Some(job) if job.parsed.is_empty() => {
                findings.push(format!(
                    "required CI job '{job_id}' has no allowlisted run: command"
                ));
            }
            Some(_) => {}
        }
    }
    findings
}

fn dangling_findings(
    citations: &[IndexedCitation],
    live: &BTreeSet<String>,
    tombs: &BTreeSet<String>,
) -> Vec<String> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for citation in citations {
        if !seen.insert((citation.id.clone(), citation.file.clone())) {
            continue;
        }
        if live.contains(&citation.id) {
            continue;
        }
        if tombs.contains(&citation.id) {
            findings.push(format!(
                "citation {} in {} points at tombstoned ID",
                citation.id, citation.file
            ));
        } else {
            findings.push(format!(
                "citation {} in {} is dangling",
                citation.id, citation.file
            ));
        }
    }
    findings
}

fn eligible_ids(
    citations: &[IndexedCitation],
    class: &ClassConfig,
    jobs: &std::collections::BTreeMap<String, JobCommands>,
    packages: &[crate::eligibility::Package],
    tracked: &BTreeSet<String>,
    public_journey_only: bool,
) -> BTreeSet<String> {
    let mut eligible = BTreeSet::new();
    for citation in citations {
        if citation.skipped {
            continue;
        }
        if !tracked.contains(&citation.file) {
            continue;
        }
        // Internal Rust source files are not public journey surfaces. In
        // particular, comments inside `#[cfg(test)] mod tests` must not
        // satisfy the repository's public e2e/journey bookend.
        if public_journey_only && is_internal_rust_source(&citation.file) {
            continue;
        }
        if class.required_ci_jobs.iter().any(|job_id| {
            jobs.get(job_id).is_some_and(|job| {
                job.parsed
                    .iter()
                    .any(|collection| collection_contains(&citation.file, collection, packages))
            })
        }) {
            eligible.insert(citation.id.clone());
        }
    }
    eligible
}

fn is_internal_rust_source(file: &str) -> bool {
    let normalized = file.replace('\\', "/");
    let mut parts = normalized.split('/');
    parts.next() == Some("crates") && parts.any(|part| part == "src")
}
