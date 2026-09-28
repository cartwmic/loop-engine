use crate::check::is_internal_rust_source;
use crate::config::{parse_repo_config, ClassConfig};
use crate::eligibility::{
    collection_contains, is_non_proof_surface, load_workflow_jobs, workspace_packages, JobCommands,
};
use crate::git::{self, TreeReader, Worktree};
use crate::prd::{has_skip_marker, parse_prd, RecordKind};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

/// Read-only, pre-plan inspection of the inputs and public collection surfaces
/// used by the ordinary Bookends checker. It does not run tests or claim that
/// a requirement has been semantically covered.
#[derive(Debug, Clone, Serialize)]
pub struct BookendsPreview {
    pub status: &'static str,
    pub schema_version: u32,
    pub evaluation_cwd: String,
    pub gate_executed: bool,
    pub tests_executed: bool,
    pub authority_note: String,
    pub bookends_toml: SourceFilePreview,
    pub prd: PrdPreview,
    pub proof_classes: Vec<ProofClassPreview>,
    pub missing_prerequisites: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceFilePreview {
    pub path: String,
    pub present: bool,
    pub text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrdPreview {
    pub path: Option<String>,
    pub present: bool,
    pub live_requirements: Vec<LiveRequirementPreview>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveRequirementPreview {
    pub id: String,
    pub title: String,
    pub coverage_classes: Vec<String>,
    pub normative_text: String,
    pub explicit_references: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProofClassPreview {
    pub name: String,
    pub pathspecs: Vec<String>,
    pub required_ci_jobs: Vec<String>,
    pub tracked_locations: Vec<ProofLocationPreview>,
    pub eligible_public_locations: Vec<String>,
    pub ci_jobs: Vec<CiJobPreview>,
    pub missing_prerequisites: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProofLocationPreview {
    pub path: String,
    pub public_source: bool,
    pub skipped: bool,
    pub collected_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CiJobPreview {
    pub id: String,
    pub found: bool,
    pub observed_run_commands: Vec<ObservedCiCommandPreview>,
    pub recognized_collections: Vec<RecognizedCollectionPreview>,
    pub eligible_locations: Vec<String>,
    pub missing_prerequisites: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservedCiCommandPreview {
    pub command: String,
    pub working_directory: String,
    pub recognized_collection: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecognizedCollectionPreview {
    pub command: String,
    pub working_directory: String,
}

/// Inspect the same working-tree configuration, PRD parser, pathspecs,
/// workspace target graph, and restricted CI command grammar as `check_repo`.
/// Missing prerequisites are observations only: no test or behavior is run.
pub fn preview_repo(repo_root: &Path) -> Result<BookendsPreview, io::Error> {
    let metadata =
        fs::metadata(repo_root).map_err(|error| crate::io_err_reading_root(repo_root, error))?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("repo root is not a directory: {}", repo_root.display()),
        ));
    }

    let mut missing = Vec::new();
    let config_file = source_file(repo_root, "bookends.toml");
    let mut parsed_config = None;
    if let Some(error) = config_file.error.as_ref() {
        missing.push(error.clone());
    } else if let Some(text) = config_file.text.as_deref() {
        match parse_repo_config(text) {
            Ok(config) => parsed_config = Some(config),
            Err(error) => missing.push(error),
        }
    } else {
        missing.push(
            "bookends.toml is missing; add repository configuration before plan approval"
                .to_owned(),
        );
    }

    let mut prd = PrdPreview {
        path: parsed_config.as_ref().map(|config| config.prd.clone()),
        present: false,
        live_requirements: Vec::new(),
        error: None,
    };
    if let Some(config) = parsed_config.as_ref() {
        match fs::read_to_string(repo_root.join(&config.prd)) {
            Ok(text) => {
                prd.present = true;
                match parse_prd(&text) {
                    Ok(parsed) => {
                        prd.live_requirements = parsed
                            .records
                            .into_iter()
                            .filter_map(|record| {
                                let RecordKind::Live { classes } = record.kind else {
                                    return None;
                                };
                                Some(LiveRequirementPreview {
                                    id: record.id,
                                    title: record.title,
                                    coverage_classes: classes
                                        .into_iter()
                                        .map(|class| class.token().to_owned())
                                        .collect(),
                                    explicit_references: extract_explicit_references(
                                        &record.text,
                                        &config.prd,
                                    ),
                                    normative_text: record.text,
                                })
                            })
                            .collect();
                        if prd.live_requirements.is_empty() {
                            let issue = format!(
                                "{} has no parsed live requirement text; add accepted live wording or plan the required owner/document work",
                                config.prd
                            );
                            prd.error = Some(issue.clone());
                            missing.push(issue);
                        }
                    }
                    Err(errors) => {
                        let issue =
                            format!("{} cannot be parsed: {}", config.prd, errors.join("; "));
                        prd.error = Some(issue.clone());
                        missing.push(issue);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let issue = format!(
                    "{} is missing; identify and plan the required accepted live requirement text",
                    config.prd
                );
                prd.error = Some(issue.clone());
                missing.push(issue);
            }
            Err(error) => {
                let issue = format!("cannot read {}: {error}", config.prd);
                prd.error = Some(issue.clone());
                missing.push(issue);
            }
        }
    }

    if !git::is_git_repo(repo_root) {
        missing.push(
            "evaluation cwd is not a Git work tree; Bookends eligibility cannot be established"
                .to_owned(),
        );
    }
    let tree = Worktree::new(repo_root);
    let tracked = match tree.tracked_files() {
        Ok(files) => files.into_iter().collect::<BTreeSet<_>>(),
        Err(error) => {
            missing.push(error);
            BTreeSet::new()
        }
    };
    let jobs = match load_workflow_jobs(&tree) {
        Ok(jobs) => Some(jobs),
        Err(error) => {
            missing.push(error);
            None
        }
    };
    let packages = match workspace_packages(&tree) {
        Ok(packages) => Some(packages),
        Err(error) => {
            missing.push(error);
            None
        }
    };

    let proof_classes = parsed_config
        .as_ref()
        .map(|config| {
            let mut classes = vec![preview_class(
                &tree,
                "e2e/journey",
                &config.e2e_journey,
                &tracked,
                jobs.as_ref(),
                packages.as_deref(),
            )];
            if let Some(contract) = config.contract.as_ref() {
                classes.push(preview_class(
                    &tree,
                    "contract",
                    contract,
                    &tracked,
                    jobs.as_ref(),
                    packages.as_deref(),
                ));
            }
            classes
        })
        .unwrap_or_default();

    for class in &proof_classes {
        missing.extend(class.missing_prerequisites.iter().cloned());
    }
    missing.sort();
    missing.dedup();

    Ok(BookendsPreview {
        status: "preview",
        schema_version: 1,
        evaluation_cwd: repo_root.to_string_lossy().into_owned(),
        gate_executed: false,
        tests_executed: false,
        authority_note: "The live status and text below reflect the selected working-tree PRD; this read-only preview does not determine owner acceptance or semantic sufficiency.".to_owned(),
        bookends_toml: config_file,
        prd,
        proof_classes,
        missing_prerequisites: missing,
    })
}

fn source_file(repo_root: &Path, relative: &str) -> SourceFilePreview {
    match fs::read_to_string(repo_root.join(relative)) {
        Ok(text) => SourceFilePreview {
            path: relative.to_owned(),
            present: true,
            text: Some(text),
            error: None,
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => SourceFilePreview {
            path: relative.to_owned(),
            present: false,
            text: None,
            error: None,
        },
        Err(error) => SourceFilePreview {
            path: relative.to_owned(),
            present: false,
            text: None,
            error: Some(format!("cannot read {relative}: {error}")),
        },
    }
}

fn preview_class(
    tree: &Worktree<'_>,
    name: &str,
    config: &ClassConfig,
    tracked: &BTreeSet<String>,
    jobs: Option<&std::collections::BTreeMap<String, JobCommands>>,
    packages: Option<&[crate::eligibility::Package]>,
) -> ProofClassPreview {
    let mut missing = Vec::new();
    let pathspec_files = match tree.pathspec_files(&config.pathspecs) {
        Ok(files) => files,
        Err(error) => {
            missing.push(error);
            Vec::new()
        }
    };
    let tracked_files: Vec<_> = pathspec_files
        .into_iter()
        .filter(|path| tracked.contains(path))
        .collect();
    if tracked_files.is_empty() {
        missing.push(format!(
            "{name} pathspecs match no tracked files; add or correct a configured public proof location"
        ));
    }

    let mut ci_jobs = Vec::new();
    for job_id in &config.required_ci_jobs {
        let mut job_preview = CiJobPreview {
            id: job_id.clone(),
            found: false,
            observed_run_commands: Vec::new(),
            recognized_collections: Vec::new(),
            eligible_locations: Vec::new(),
            missing_prerequisites: Vec::new(),
        };
        match jobs.and_then(|jobs| jobs.get(job_id)) {
            None => {
                let issue = format!(
                    "required CI job '{job_id}' was not found or could not be inspected; configure a required workflow job"
                );
                job_preview.missing_prerequisites.push(issue.clone());
                missing.push(issue);
            }
            Some(job) => {
                job_preview.found = true;
                job_preview.observed_run_commands = job
                    .observed
                    .iter()
                    .map(|run| ObservedCiCommandPreview {
                        command: run.command.clone(),
                        working_directory: run.working_directory.clone(),
                        recognized_collection: run
                            .recognized_collection
                            .as_ref()
                            .map(crate::eligibility::Collection::command),
                    })
                    .collect();
                job_preview.recognized_collections = job
                    .parsed
                    .iter()
                    .map(|collection| RecognizedCollectionPreview {
                        command: collection.command(),
                        working_directory:
                            "repository root (effective cwd accepted by Bookends grammar)"
                                .to_owned(),
                    })
                    .collect();
                if job_preview.recognized_collections.is_empty() {
                    let issue = format!(
                        "required CI job '{job_id}' has no recognized root-cwd collection; use an accepted Bookends collection command at repository root"
                    );
                    job_preview.missing_prerequisites.push(issue.clone());
                    missing.push(issue);
                }
                if let (Some(packages), Some(jobs)) = (packages, jobs) {
                    job_preview.eligible_locations = tracked_files
                        .iter()
                        .filter(|file| is_eligible_location(name, file, job_id, jobs, packages))
                        .cloned()
                        .collect();
                }
            }
        }
        ci_jobs.push(job_preview);
    }

    let locations: Vec<_> = tracked_files
        .iter()
        .map(|path| {
            let text = tree.read_text(path).ok().flatten().unwrap_or_default();
            let public_source = !is_non_proof_surface(path)
                && !(name == "e2e/journey" && is_internal_rust_source(path));
            let skipped = has_skip_marker(&text);
            let collected_by = match (jobs, packages) {
                (Some(jobs), Some(packages)) => config
                    .required_ci_jobs
                    .iter()
                    .filter(|job_id| is_eligible_location(name, path, job_id, jobs, packages))
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            };
            ProofLocationPreview {
                path: path.clone(),
                public_source,
                skipped,
                collected_by,
            }
        })
        .collect();
    let eligible_public_locations: Vec<_> = locations
        .iter()
        .filter(|location| {
            location.public_source && !location.skipped && !location.collected_by.is_empty()
        })
        .map(|location| location.path.clone())
        .collect();
    if eligible_public_locations.is_empty() {
        missing.push(format!(
            "{name} has no tracked eligible public location collected by its required CI jobs; plan a recognized public proof/CI collection"
        ));
    }

    ProofClassPreview {
        name: name.to_owned(),
        pathspecs: config.pathspecs.clone(),
        required_ci_jobs: config.required_ci_jobs.clone(),
        tracked_locations: locations,
        eligible_public_locations,
        ci_jobs,
        missing_prerequisites: missing,
    }
}

fn is_eligible_location(
    class_name: &str,
    file: &str,
    job_id: &str,
    jobs: &std::collections::BTreeMap<String, JobCommands>,
    packages: &[crate::eligibility::Package],
) -> bool {
    if is_non_proof_surface(file) || (class_name == "e2e/journey" && is_internal_rust_source(file))
    {
        return false;
    }
    jobs.get(job_id).is_some_and(|job| {
        job.parsed
            .iter()
            .any(|collection| collection_contains(file, collection, packages))
    })
}

fn extract_explicit_references(text: &str, prd_path: &str) -> Vec<String> {
    let mut references = BTreeSet::new();
    let mut plain_text = String::new();
    let mut cursor = 0;
    while let Some(relative_start) = text[cursor..].find('[') {
        let start = cursor + relative_start;
        let Some(relative_link) = text[start..].find("](") else {
            plain_text.push_str(&text[cursor..]);
            cursor = text.len();
            break;
        };
        let link_start = start + relative_link;
        let Some(relative_end) = text[link_start + 2..].find(')') else {
            plain_text.push_str(&text[cursor..]);
            cursor = text.len();
            break;
        };
        let link_end = link_start + 2 + relative_end;
        plain_text.push_str(&text[cursor..start]);
        plain_text.push(' ');
        let destination = text[link_start + 2..link_end]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|character| matches!(character, '<' | '>'));
        if !destination.is_empty() {
            references.insert(resolve_markdown_reference(prd_path, destination));
        }
        cursor = link_end + 1;
    }
    if cursor < text.len() {
        plain_text.push_str(&text[cursor..]);
    }

    for marker in [
        "docs/",
        "crates/",
        "scripts/",
        ".github/workflows/",
        "bookends.toml",
    ] {
        let mut remaining = plain_text.as_str();
        while let Some(index) = remaining.find(marker) {
            let suffix = &remaining[index..];
            let end = suffix
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, ']' | '(' | ')' | ',' | ';' | ':' | '}')
                })
                .unwrap_or(suffix.len());
            let candidate = suffix[..end]
                .trim_matches(|character| matches!(character, '`' | '*' | '_' | '{' | '.'));
            if !candidate.is_empty() {
                references.insert(candidate.to_owned());
            }
            remaining = &suffix[marker.len().min(suffix.len())..];
        }
    }
    references.into_iter().collect()
}

fn resolve_markdown_reference(prd_path: &str, destination: &str) -> String {
    if destination.starts_with('#') || destination.contains("://") || destination.starts_with('/') {
        return destination.to_owned();
    }
    let (path, fragment) = destination
        .split_once('#')
        .map(|(path, fragment)| (path, Some(fragment)))
        .unwrap_or((destination, None));
    let mut components = std::path::PathBuf::from(prd_path)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    for component in Path::new(path).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if components.last().is_some_and(|part| part != "..") {
                    components.pop();
                } else {
                    components.push("..".into());
                }
            }
            std::path::Component::Normal(part) => components.push(part.to_os_string()),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return destination.to_owned();
            }
        }
    }
    let mut result = std::path::PathBuf::new();
    for component in components {
        result.push(component);
    }
    let mut result = result.to_string_lossy().replace('\\', "/");
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn preplan_preview_reports_live_text_and_cwd_qualified_collection_without_running_tests() {
        let root = std::env::temp_dir().join(format!(
            "bookends-preview-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("scripts")).expect("root");
        fs::create_dir_all(root.join("docs")).expect("docs");
        fs::create_dir_all(root.join(".github/workflows")).expect("workflows");
        fs::write(
            root.join("bookends.toml"),
            "prd = \"docs/PRD.md\"\n\n[classes.e2e_journey]\npathspecs = [\"scripts/**\"]\nrequired_ci_jobs = [\"journey\"]\n",
        )
        .expect("config");
        fs::write(
            root.join("docs/PRD.md"),
            "### LE-1: Testable outcome\n- Status: live\n- Coverage: e2e/journey\n\nSee [design](../docs/design.md).\n",
        )
        .expect("PRD");
        fs::write(root.join("scripts/journey.py"), "# assertions\n").expect("script");
        fs::write(
            root.join(".github/workflows/test.yml"),
            "jobs:\n  journey:\n    steps:\n      - run: python3 scripts/journey.py\n",
        )
        .expect("workflow");
        for args in [
            vec!["init", "-q"],
            vec![
                "add",
                "bookends.toml",
                "docs/PRD.md",
                "scripts/journey.py",
                ".github/workflows/test.yml",
            ],
        ] {
            let status = Command::new("git")
                .args(args)
                .current_dir(&root)
                .status()
                .expect("git available");
            assert!(status.success());
        }
        let report = preview_repo(&root).expect("preview");
        assert!(!report.gate_executed);
        assert!(!report.tests_executed);
        assert_eq!(report.prd.live_requirements.len(), 1);
        assert_eq!(report.prd.live_requirements[0].id, "LE-1");
        assert!(report.prd.live_requirements[0]
            .explicit_references
            .contains(&"docs/design.md".to_owned()));
        assert_eq!(
            report.proof_classes[0].eligible_public_locations,
            ["scripts/journey.py"]
        );
        assert!(
            report.missing_prerequisites.is_empty(),
            "{:?}",
            report.missing_prerequisites
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
