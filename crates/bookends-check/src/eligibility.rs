use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::git;
use crate::prd::{has_skip_marker, scan_citation_tokens};

/// The only CI collection forms used by the adopting repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Collection {
    WorkspaceRustTests,
    SingleFile(String),
}

#[derive(Debug, Clone)]
pub(crate) struct JobCommands {
    pub parsed: Vec<Collection>,
}

pub(crate) fn load_workflow_jobs(repo: &Path) -> Result<BTreeMap<String, JobCommands>, String> {
    load_workflow_jobs_worktree(repo)
}

fn load_workflow_jobs_worktree(repo: &Path) -> Result<BTreeMap<String, JobCommands>, String> {
    let dir = repo.join(".github").join("workflows");
    if !dir.is_dir() {
        return Ok(BTreeMap::new());
    }
    let tracked: BTreeSet<String> = git::tracked_files(repo)?.into_iter().collect();
    let mut jobs = BTreeMap::new();
    let entries = fs::read_dir(&dir).map_err(|err| format!("read {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".yml") {
            continue;
        }
        let rel = format!(".github/workflows/{name}");
        if !tracked.contains(&rel) {
            continue;
        }
        let text =
            fs::read_to_string(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        merge_jobs(&mut jobs, parse_workflow_jobs(&text, name)?);
    }
    Ok(jobs)
}

fn merge_jobs(dst: &mut BTreeMap<String, JobCommands>, src: BTreeMap<String, JobCommands>) {
    for (id, job) in src {
        dst.entry(id)
            .and_modify(|existing| {
                existing.parsed.extend(job.parsed.iter().cloned());
            })
            .or_insert(job);
    }
}

fn parse_workflow_jobs(
    text: &str,
    filename: &str,
) -> Result<BTreeMap<String, JobCommands>, String> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(text)
        .map_err(|err| format!("parse .github/workflows/{filename}: {err}"))?;
    let mut out = BTreeMap::new();
    let Some(jobs) = yaml.get("jobs").and_then(|value| value.as_mapping()) else {
        return Ok(out);
    };
    for (key, job) in jobs {
        let Some(id) = yaml_key(key) else {
            continue;
        };
        let mut parsed = Vec::new();
        if let Some(steps) = job.get("steps").and_then(|value| value.as_sequence()) {
            for step in steps {
                let Some(run) = step.get("run").and_then(|value| value.as_str()) else {
                    continue;
                };
                if !cwd_is_repo_root(&yaml, job, step) {
                    continue;
                }
                if let Some(collection) = parse_run_command(run) {
                    parsed.push(collection);
                }
            }
        }
        out.insert(id, JobCommands { parsed });
    }
    Ok(out)
}

fn yaml_key(key: &serde_yaml::Value) -> Option<String> {
    match key {
        serde_yaml::Value::String(value) => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        serde_yaml::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

/// Parse one complete `run:` scalar.  Deliberately do not accept shell
/// wrappers, flags, package filters, or multi-command scripts: only the two
/// explicit collection forms can establish default required-CI eligibility.
pub(crate) fn parse_run_command(run: &str) -> Option<Collection> {
    let argv: Vec<&str> = run.split_whitespace().collect();
    match argv.as_slice() {
        ["cargo", "test", "--workspace"] => Some(Collection::WorkspaceRustTests),
        ["python3", script] if is_repo_relative_script(script) => {
            Some(Collection::SingleFile(script.replace('\\', "/")))
        }
        _ => None,
    }
}

fn is_repo_relative_script(script: &str) -> bool {
    let normalized = script.replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && !normalized.contains("://")
        && !normalized
            .split('/')
            .any(|part| part.is_empty() || part == "..")
        && !normalized.starts_with('-')
}

enum Workdir<'a> {
    Set(&'a str),
    Foreign,
}

fn cwd_is_repo_root(
    workflow: &serde_yaml::Value,
    job: &serde_yaml::Value,
    step: &serde_yaml::Value,
) -> bool {
    match working_directory(step)
        .or_else(|| run_defaults(job))
        .or_else(|| run_defaults(workflow))
    {
        None => true,
        Some(Workdir::Foreign) => false,
        Some(Workdir::Set(value)) => is_repo_root_workdir(value),
    }
}

fn working_directory(node: &serde_yaml::Value) -> Option<Workdir<'_>> {
    match node.get("working-directory") {
        None => None,
        Some(serde_yaml::Value::String(value)) => Some(Workdir::Set(value)),
        Some(_) => Some(Workdir::Foreign),
    }
}

fn run_defaults(node: &serde_yaml::Value) -> Option<Workdir<'_>> {
    working_directory(node.get("defaults")?.get("run")?)
}

fn is_repo_root_workdir(value: &str) -> bool {
    let normalized = value.trim().replace('\\', "/");
    normalized.trim_end_matches('/') == "."
}

#[derive(Debug, Clone)]
pub(crate) struct Package {
    pub dir: String,
    targets: Vec<RustTarget>,
}

#[derive(Debug, Clone)]
struct RustTarget {
    path: String,
    root: String,
    kind: TargetKind,
}

#[derive(Debug, Clone, Copy)]
enum TargetKind {
    Library,
    Binary,
    Integration,
}

/// Read enough Cargo metadata to exclude targets that `cargo test
/// --workspace` does not execute by default.  This is intentionally a
/// workspace-only projection, not a general runner or manifest interpreter.
pub(crate) fn workspace_packages(repo: &Path) -> Result<Vec<Package>, String> {
    let Some(text) = git::read_text(repo, "Cargo.toml")? else {
        return Ok(Vec::new());
    };
    let root: toml::Value =
        toml::from_str(&text).map_err(|err| format!("parse Cargo.toml: {err}"))?;
    let tracked = git::tracked_files(repo)?;
    let mut dirs = Vec::new();
    if let Some(members) = root
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(toml::Value::as_array)
    {
        for member in members.iter().filter_map(toml::Value::as_str) {
            dirs.extend(expand_member(&tracked, member));
        }
    }
    if root.get("package").is_some() {
        dirs.push(String::new());
    }
    dirs.sort();
    dirs.dedup();

    let mut packages = Vec::new();
    for dir in dirs {
        let manifest_path = if dir.is_empty() {
            "Cargo.toml".to_owned()
        } else {
            format!("{dir}/Cargo.toml")
        };
        let Some(text) = git::read_text(repo, &manifest_path)? else {
            continue;
        };
        let manifest: toml::Value =
            toml::from_str(&text).map_err(|err| format!("parse {manifest_path}: {err}"))?;
        packages.push(Package {
            targets: package_targets(&manifest, &dir, &tracked)?,
            dir,
        });
    }
    Ok(packages)
}

fn package_targets(
    manifest: &toml::Value,
    package_dir: &str,
    tracked: &[String],
) -> Result<Vec<RustTarget>, String> {
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or("missing [package] table")?;
    let auto_lib = bool_field(package, "autolib", true)?;
    let auto_bins = bool_field(package, "autobins", true)?;
    let auto_tests = bool_field(package, "autotests", true)?;
    let mut targets = Vec::new();

    let explicit_lib = manifest.get("lib").and_then(toml::Value::as_table);
    if explicit_lib.is_some() || (auto_lib && tracked_contains(tracked, package_dir, "src/lib.rs"))
    {
        if let Some(table) = explicit_lib {
            if bool_field(table, "test", true)? {
                targets.push(rust_target(
                    target_path(table, "src/lib.rs")?,
                    TargetKind::Library,
                ));
            }
        } else {
            targets.push(rust_target("src/lib.rs".to_owned(), TargetKind::Library));
        }
    }

    let mut explicit_bins = BTreeSet::new();
    if let Some(bins) = manifest.get("bin").and_then(toml::Value::as_array) {
        for bin in bins {
            let table = bin.as_table().ok_or("[[bin]] must be a table")?;
            let path = target_path_with_default(table, "src/bin", ".rs")?;
            explicit_bins.insert(path.clone());
            if bool_field(table, "test", true)? {
                targets.push(rust_target(path, TargetKind::Binary));
            }
        }
    }
    if auto_bins {
        if tracked_contains(tracked, package_dir, "src/main.rs")
            && !explicit_bins.contains("src/main.rs")
        {
            targets.push(rust_target("src/main.rs".to_owned(), TargetKind::Binary));
        }
        for path in tracked_package_files(tracked, package_dir) {
            let Some(name) = path.strip_prefix("src/bin/") else {
                continue;
            };
            if name.ends_with(".rs")
                && (!name.contains('/') || name.ends_with("/main.rs"))
                && !explicit_bins.contains(&path)
            {
                targets.push(rust_target(path, TargetKind::Binary));
            }
        }
    }

    let mut explicit_tests = BTreeSet::new();
    if let Some(tests) = manifest.get("test").and_then(toml::Value::as_array) {
        for test in tests {
            let table = test.as_table().ok_or("[[test]] must be a table")?;
            let path = target_path_with_default(table, "tests", ".rs")?;
            explicit_tests.insert(path.clone());
            if bool_field(table, "test", true)? {
                targets.push(rust_target(path, TargetKind::Integration));
            }
        }
    }
    if auto_tests {
        for path in tracked_package_files(tracked, package_dir) {
            let Some(name) = path.strip_prefix("tests/") else {
                continue;
            };
            if name.ends_with(".rs")
                && (!name.contains('/') || name.ends_with("/main.rs"))
                && !explicit_tests.contains(&path)
            {
                targets.push(rust_target(path, TargetKind::Integration));
            }
        }
    }
    Ok(targets)
}

fn bool_field(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match table.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("{key} must be a boolean")),
    }
}

fn target_path(
    table: &toml::map::Map<String, toml::Value>,
    default: &str,
) -> Result<String, String> {
    match table.get("path") {
        None => Ok(default.to_owned()),
        Some(value) => value
            .as_str()
            .map(normalize_rel)
            .ok_or_else(|| "target path must be a string".to_owned()),
    }
}

fn target_path_with_default(
    table: &toml::map::Map<String, toml::Value>,
    prefix: &str,
    suffix: &str,
) -> Result<String, String> {
    match table.get("path") {
        Some(value) => value
            .as_str()
            .map(normalize_rel)
            .ok_or_else(|| "target path must be a string".to_owned()),
        None => {
            let name = table
                .get("name")
                .and_then(toml::Value::as_str)
                .ok_or("target without a name or path")?;
            Ok(format!("{prefix}/{name}{suffix}"))
        }
    }
}

fn rust_target(path: String, kind: TargetKind) -> RustTarget {
    let path = normalize_rel(&path);
    let root = match kind {
        TargetKind::Library => path
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_owned())
            .unwrap_or_default(),
        TargetKind::Binary => {
            let parent = path
                .rsplit_once('/')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            let file = path.rsplit('/').next().unwrap_or(&path);
            if file == "main.rs" {
                parent.to_owned()
            } else {
                format!("{parent}/{}", file.trim_end_matches(".rs"))
                    .trim_start_matches('/')
                    .to_owned()
            }
        }
        TargetKind::Integration => path
            .rsplit_once('/')
            .map(|(parent, file)| {
                if file == "main.rs" {
                    parent.to_owned()
                } else {
                    format!("{parent}/{}", file.trim_end_matches(".rs"))
                }
            })
            .unwrap_or_else(|| path.trim_end_matches(".rs").to_owned()),
    };
    RustTarget { path, root, kind }
}

fn tracked_contains(tracked: &[String], package_dir: &str, relative: &str) -> bool {
    tracked_package_files(tracked, package_dir)
        .iter()
        .any(|path| path == relative)
}

fn tracked_package_files(tracked: &[String], package_dir: &str) -> Vec<String> {
    let prefix = if package_dir.is_empty() {
        String::new()
    } else {
        format!("{package_dir}/")
    };
    tracked
        .iter()
        .filter_map(|path| path.strip_prefix(&prefix))
        .map(str::to_owned)
        .collect()
}

fn expand_member(tracked: &[String], spec: &str) -> Vec<String> {
    let spec = spec.replace('\\', "/");
    if !spec.contains('*') {
        return vec![spec];
    }
    let mut dirs = BTreeSet::new();
    for file in tracked.iter().filter(|file| file.ends_with("Cargo.toml")) {
        let Some(dir) = file.strip_suffix("/Cargo.toml") else {
            continue;
        };
        if glob_member_matches(&spec, dir) {
            dirs.insert(dir.to_owned());
        }
    }
    dirs.into_iter().collect()
}

fn glob_member_matches(spec: &str, dir: &str) -> bool {
    match_parts(
        &spec.split('/').collect::<Vec<_>>(),
        &dir.split('/').collect::<Vec<_>>(),
    )
}

fn match_parts(spec: &[&str], dir: &[&str]) -> bool {
    if spec.is_empty() {
        return dir.is_empty();
    }
    match spec[0] {
        "**" => spec.len() == 1 || (0..=dir.len()).any(|i| match_parts(&spec[1..], &dir[i..])),
        "*" => !dir.is_empty() && match_parts(&spec[1..], &dir[1..]),
        part => dir.first().copied() == Some(part) && match_parts(&spec[1..], &dir[1..]),
    }
}

pub(crate) fn collection_contains(
    file: &str,
    collection: &Collection,
    packages: &[Package],
) -> bool {
    match collection {
        Collection::SingleFile(script) => normalize_rel(file) == normalize_rel(script),
        Collection::WorkspaceRustTests => packages
            .iter()
            .any(|package| is_default_rust_test_file(file, package)),
    }
}

fn is_default_rust_test_file(file: &str, package: &Package) -> bool {
    let file = normalize_rel(file);
    if !file.ends_with(".rs") {
        return false;
    }
    let relative = if package.dir.is_empty() {
        file.as_str()
    } else {
        let prefix = format!("{}/", package.dir);
        match file.strip_prefix(&prefix) {
            Some(relative) => relative,
            None => return false,
        }
    };
    package
        .targets
        .iter()
        .any(|target| target_contains(relative, target))
}

fn target_contains(file: &str, target: &RustTarget) -> bool {
    if file == target.path {
        return true;
    }
    if target.root.is_empty() || !file.starts_with(&format!("{}/", target.root)) {
        return false;
    }
    match target.kind {
        TargetKind::Library => {
            target.root != "src" || (file != "src/main.rs" && !file.starts_with("src/bin/"))
        }
        TargetKind::Binary => {
            target.root != "src" || (file != "src/lib.rs" && !file.starts_with("src/bin/"))
        }
        TargetKind::Integration => true,
    }
}

fn normalize_rel(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedCitation {
    pub id: String,
    pub file: String,
    pub skipped: bool,
}

pub(crate) fn index_class_files(
    repo: &Path,
    files: &[String],
) -> Result<(Vec<IndexedCitation>, Vec<String>), Vec<String>> {
    let mut citations = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        let text = match git::read_text(repo, file) {
            Ok(Some(text)) => text,
            Ok(None) => {
                errors.push(format!("cannot read tracked file {file}"));
                continue;
            }
            Err(err) => {
                errors.push(format!("cannot read tracked file {file}: {err}"));
                continue;
            }
        };
        let skipped = has_skip_marker(&text);
        match scan_citation_tokens(&text) {
            Ok(ids) => citations.extend(ids.into_iter().map(|id| IndexedCitation {
                id,
                file: file.clone(),
                skipped,
            })),
            Err(err) => errors.extend(err.into_iter().map(|error| format!("{file}: {error}"))),
        }
    }
    if errors.is_empty() {
        Ok((citations, Vec::new()))
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_workspace_command_parses() {
        assert_eq!(
            parse_run_command("cargo test --workspace"),
            Some(Collection::WorkspaceRustTests)
        );
    }

    #[test]
    fn exact_python_command_parses() {
        assert_eq!(
            parse_run_command("python3 scripts/journey.py"),
            Some(Collection::SingleFile("scripts/journey.py".into()))
        );
    }

    #[test]
    fn extra_words_and_wrappers_are_unparsed() {
        for command in [
            "cargo test",
            "cargo test --workspace --locked",
            "cargo test --workspace --bins",
            "python3 scripts/journey.py --self-test",
            "bash -lc 'cargo test --workspace'",
        ] {
            assert_eq!(parse_run_command(command), None, "{command}");
        }
    }

    #[test]
    fn unsafe_python_paths_are_unparsed() {
        assert_eq!(parse_run_command("python3 ../journey.py"), None);
        assert_eq!(parse_run_command("python3 /tmp/journey.py"), None);
        assert_eq!(parse_run_command("python3 -m journey"), None);
    }

    #[test]
    fn nested_working_directory_is_unparsed() {
        let jobs = parse_workflow_jobs(
            "jobs:\n  test:\n    steps:\n      - run: cargo test --workspace\n        working-directory: nested\n",
            "ci.yml",
        )
        .unwrap();
        let job = jobs.get("test").unwrap();
        assert!(job.parsed.is_empty());
    }

    #[test]
    fn repo_root_working_directory_parses() {
        let jobs = parse_workflow_jobs(
            "jobs:\n  test:\n    steps:\n      - run: cargo test --workspace\n        working-directory: .\n",
            "ci.yml",
        )
        .unwrap();
        assert_eq!(jobs["test"].parsed, vec![Collection::WorkspaceRustTests]);
    }
}
