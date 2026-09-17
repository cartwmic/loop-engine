use crate::git::TreeReader;
use crate::prd::{has_skip_marker, scan_citation_tokens};
use std::collections::{BTreeMap, BTreeSet};

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

pub(crate) fn load_workflow_jobs<T: TreeReader>(
    tree: &T,
) -> Result<BTreeMap<String, JobCommands>, String> {
    let tracked = tree.tracked_files()?;
    let mut jobs = BTreeMap::new();
    for rel in tracked.iter().filter(|path| {
        path.strip_prefix(".github/workflows/")
            .is_some_and(|name| !name.contains('/') && name.ends_with(".yml"))
    }) {
        let Some(name) = rel.strip_prefix(".github/workflows/") else {
            continue;
        };
        let Some(text) = tree.read_text(rel)? else {
            return Err(format!("cannot read tracked file {rel}"));
        };
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
    targets: Vec<RustTarget>,
}

#[derive(Debug, Clone)]
struct RustTarget {
    /// The exact source files reachable from this Cargo target's root. A
    /// directory prefix is not a collection: Rust only compiles files linked
    /// by a module declaration (or an explicit `#[path]`).
    files: BTreeSet<String>,
}

/// Read enough Cargo metadata to exclude targets that `cargo test
/// --workspace` does not execute by default.  This is intentionally a
/// workspace-only projection, not a general runner or manifest interpreter.
pub(crate) fn workspace_packages<T: TreeReader>(tree: &T) -> Result<Vec<Package>, String> {
    let Some(text) = tree.read_text("Cargo.toml")? else {
        return Ok(Vec::new());
    };
    let root: toml::Value =
        toml::from_str(&text).map_err(|err| format!("parse Cargo.toml: {err}"))?;
    let tracked = tree.tracked_files()?;
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
        let Some(text) = tree.read_text(&manifest_path)? else {
            continue;
        };
        let manifest: toml::Value =
            toml::from_str(&text).map_err(|err| format!("parse {manifest_path}: {err}"))?;
        packages.push(Package {
            targets: package_targets(tree, &manifest, &dir, &tracked)?,
        });
    }
    Ok(packages)
}

fn package_targets<T: TreeReader>(
    tree: &T,
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
                    tree,
                    package_dir,
                    target_path(table, "src/lib.rs")?,
                )?);
            }
        } else {
            targets.push(rust_target(tree, package_dir, "src/lib.rs".to_owned())?);
        }
    }

    let mut explicit_bins = BTreeSet::new();
    if let Some(bins) = manifest.get("bin").and_then(toml::Value::as_array) {
        for bin in bins {
            let table = bin.as_table().ok_or("[[bin]] must be a table")?;
            let path = target_path_with_default(table, "src/bin", ".rs")?;
            explicit_bins.insert(path.clone());
            if bool_field(table, "test", true)? {
                targets.push(rust_target(tree, package_dir, path)?);
            }
        }
    }
    if auto_bins {
        if tracked_contains(tracked, package_dir, "src/main.rs")
            && !explicit_bins.contains("src/main.rs")
        {
            targets.push(rust_target(tree, package_dir, "src/main.rs".to_owned())?);
        }
        for path in tracked_package_files(tracked, package_dir) {
            let Some(name) = path.strip_prefix("src/bin/") else {
                continue;
            };
            if name.ends_with(".rs")
                && (!name.contains('/') || name.ends_with("/main.rs"))
                && !explicit_bins.contains(&path)
            {
                targets.push(rust_target(tree, package_dir, path)?);
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
                targets.push(rust_target(tree, package_dir, path)?);
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
                targets.push(rust_target(tree, package_dir, path)?);
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

fn rust_target<T: TreeReader>(
    tree: &T,
    package_dir: &str,
    path: String,
) -> Result<RustTarget, String> {
    let path = join_module_path(package_dir, &normalize_rel(&path))
        .ok_or_else(|| format!("Rust target path escapes the workspace: {package_dir}/{path}"))?;
    Ok(RustTarget {
        files: collect_rust_modules(tree, &path)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RustToken {
    Ident(String),
    String(String),
    Punct(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalModule {
    name: String,
    path: Option<String>,
    /// The directory against which this declaration resolves. Ordinary
    /// modules use the logical module directory; `#[path]` uses the physical
    /// directory of the containing source file.
    module_dir: String,
}

/// Follow only source files linked by Rust's ordinary external-module forms.
/// This deliberately does not try to classify tests or interpret macros: the
/// collection question is lexical linkage, while whether an assertion is a
/// good proof remains a review concern.
fn collect_rust_modules<T: TreeReader>(tree: &T, root: &str) -> Result<BTreeSet<String>, String> {
    let mut files = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let root = normalize_rel(root);
    collect_rust_module(tree, &root, &parent_dir(&root), &mut visited, &mut files)?;
    Ok(files)
}

fn collect_rust_module<T: TreeReader>(
    tree: &T,
    file: &str,
    module_dir: &str,
    visited: &mut BTreeSet<(String, String)>,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    let file = normalize_rel(file);
    let module_dir = normalize_rel(module_dir);
    if !visited.insert((file.clone(), module_dir.clone())) {
        return Ok(());
    }
    let Some(text) = tree.read_text(&file)? else {
        // Cargo would reject a missing module during compilation.  It is not
        // a proof file, though, so leave it out of the collection rather than
        // treating an absent source blob as a directory-wide match.
        return Ok(());
    };
    files.insert(file.clone());

    for declaration in external_modules(&text, &module_dir, &parent_dir(&file)) {
        let Some(child) = resolve_module_file(tree, &declaration)? else {
            continue;
        };
        let child_dir = if declaration.path.is_some() {
            // Rust's #[path] module is rooted at the physical directory that
            // contains the selected file.  This is what lets the central
            // workspace test target import another test root and retain that
            // root's own `mod common;` linkage.
            parent_dir(&child)
        } else {
            normal_module_dir(&child)
        };
        collect_rust_module(tree, &child, &child_dir, visited, files)?;
    }
    Ok(())
}

fn resolve_module_file<T: TreeReader>(
    tree: &T,
    declaration: &ExternalModule,
) -> Result<Option<String>, String> {
    let candidates = if let Some(path) = &declaration.path {
        vec![join_module_path(
            &declaration.module_dir,
            &normalize_rel(path),
        )]
    } else {
        vec![
            join_module_path(&declaration.module_dir, &format!("{}.rs", declaration.name)),
            join_module_path(
                &declaration.module_dir,
                &format!("{}/mod.rs", declaration.name),
            ),
        ]
    };
    for candidate in candidates.into_iter().flatten() {
        if tree.read_text(&candidate)?.is_some() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn external_modules(text: &str, module_dir: &str, file_dir: &str) -> Vec<ExternalModule> {
    let tokens = rust_tokens(text);
    let mut modules = Vec::new();
    scan_module_items(&tokens, 0, tokens.len(), module_dir, file_dir, &mut modules);
    modules
}

fn scan_module_items(
    tokens: &[RustToken],
    start: usize,
    end: usize,
    module_dir: &str,
    file_dir: &str,
    modules: &mut Vec<ExternalModule>,
) {
    let mut index = start;
    while index + 2 < end {
        if matches!(tokens.get(index), Some(RustToken::Ident(value)) if value == "mod")
            && matches!(tokens.get(index + 1), Some(RustToken::Ident(_)))
        {
            let Some(RustToken::Ident(name)) = tokens.get(index + 1) else {
                index += 1;
                continue;
            };
            match tokens.get(index + 2) {
                Some(RustToken::Punct(';')) => {
                    let path = path_attribute_before(tokens, index);
                    modules.push(ExternalModule {
                        name: name.clone(),
                        module_dir: if path.is_some() {
                            file_dir.to_owned()
                        } else {
                            module_dir.to_owned()
                        },
                        path,
                    });
                    index += 3;
                    continue;
                }
                Some(RustToken::Punct('{')) => {
                    if let Some(close) = matching_brace(tokens, index + 2, end) {
                        let inline_dir = join_module_path(module_dir, name)
                            .unwrap_or_else(|| module_dir.to_owned());
                        scan_module_items(tokens, index + 3, close, &inline_dir, file_dir, modules);
                        index = close + 1;
                        continue;
                    }
                }
                _ => {}
            }
        }
        if matches!(tokens.get(index), Some(RustToken::Punct('{'))) {
            index = matching_brace(tokens, index, end)
                .map(|close| close + 1)
                .unwrap_or(end);
        } else {
            index += 1;
        }
    }
}

fn matching_brace(tokens: &[RustToken], open: usize, end: usize) -> Option<usize> {
    let mut depth = 0;
    for index in open..end {
        match tokens.get(index) {
            Some(RustToken::Punct('{')) => depth += 1,
            Some(RustToken::Punct('}')) => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn path_attribute_before(tokens: &[RustToken], module_index: usize) -> Option<String> {
    let mut cursor = module_index;
    let mut path = None;
    while cursor > 0 && matches!(tokens.get(cursor - 1), Some(RustToken::Punct(']'))) {
        let close = cursor - 1;
        let open = matching_open_bracket(tokens, close)?;
        if open == 0 || !matches!(tokens.get(open - 1), Some(RustToken::Punct('#'))) {
            break;
        }
        if let Some(value) = path_attribute_value(&tokens[open + 1..close]) {
            path = Some(value);
        }
        cursor = open - 1;
    }
    path
}

fn matching_open_bracket(tokens: &[RustToken], close: usize) -> Option<usize> {
    let mut depth = 0;
    for index in (0..=close).rev() {
        match tokens.get(index) {
            Some(RustToken::Punct(']')) => depth += 1,
            Some(RustToken::Punct('[')) => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn path_attribute_value(tokens: &[RustToken]) -> Option<String> {
    match tokens {
        [RustToken::Ident(name), RustToken::Punct('='), RustToken::String(value)]
            if name == "path" =>
        {
            Some(value.clone())
        }
        _ => None,
    }
}

fn rust_tokens(text: &str) -> Vec<RustToken> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            index = skip_line_comment(bytes, index + 2);
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index = skip_block_comment(bytes, index + 2);
            continue;
        }
        if let Some((value, next)) = raw_string(bytes, index) {
            tokens.push(RustToken::String(value));
            index = next;
            continue;
        }
        if bytes[index] == b'"' {
            let (value, next) = quoted_string(bytes, index, b'"');
            tokens.push(RustToken::String(value));
            index = next;
            continue;
        }
        if bytes[index] == b'\'' {
            if let Some(next) = char_literal_end(bytes, index) {
                index = next;
                continue;
            }
            index += 1;
            continue;
        }
        if is_ascii_ident_start(bytes[index]) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_ascii_ident_continue(bytes[index]) {
                index += 1;
            }
            tokens.push(RustToken::Ident(
                String::from_utf8_lossy(&bytes[start..index]).into_owned(),
            ));
            continue;
        }
        if matches!(
            bytes[index],
            b'#' | b'[' | b']' | b'(' | b')' | b'{' | b'}' | b'=' | b';'
        ) {
            tokens.push(RustToken::Punct(bytes[index] as char));
        }
        index += 1;
    }
    tokens
}

fn skip_line_comment(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1;
    while index < bytes.len() && depth > 0 {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            depth += 1;
            index += 2;
        } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
            depth -= 1;
            index += 2;
        } else {
            index += 1;
        }
    }
    index
}

fn raw_string(bytes: &[u8], index: usize) -> Option<(String, usize)> {
    let mut marker = index;
    if bytes.get(marker) == Some(&b'b') {
        marker += 1;
    }
    if bytes.get(marker) != Some(&b'r') {
        return None;
    }
    marker += 1;
    let mut hashes = 0;
    while bytes.get(marker) == Some(&b'#') {
        hashes += 1;
        marker += 1;
    }
    if bytes.get(marker) != Some(&b'"') {
        return None;
    }
    let content_start = marker + 1;
    let mut cursor = content_start;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes
                .get(cursor + 1..cursor + 1 + hashes)
                .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
        {
            let end = cursor + 1 + hashes;
            return Some((
                String::from_utf8_lossy(&bytes[content_start..cursor]).into_owned(),
                end,
            ));
        }
        cursor += 1;
    }
    Some((
        String::from_utf8_lossy(&bytes[content_start..]).into_owned(),
        bytes.len(),
    ))
}

fn quoted_string(bytes: &[u8], index: usize, quote: u8) -> (String, usize) {
    let mut value = Vec::new();
    let mut cursor = index + 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            byte if byte == quote => {
                return (String::from_utf8_lossy(&value).into_owned(), cursor + 1)
            }
            b'\\' if cursor + 1 < bytes.len() => {
                value.push(match bytes[cursor + 1] {
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    other => other,
                });
                cursor += 2;
            }
            byte => {
                value.push(byte);
                cursor += 1;
            }
        }
    }
    (String::from_utf8_lossy(&value).into_owned(), bytes.len())
}

fn char_literal_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut cursor = index + 1;
    if cursor >= bytes.len() || bytes[cursor] == b'\n' || is_ascii_ident_continue(bytes[cursor]) {
        return None;
    }
    while cursor < bytes.len() && bytes[cursor] != b'\n' {
        if bytes[cursor] == b'\\' {
            cursor = cursor.saturating_add(2);
        } else if bytes[cursor] == b'\'' {
            return Some(cursor + 1);
        } else {
            cursor += 1;
        }
    }
    None
}

fn is_ascii_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ascii_ident_continue(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn parent_dir(path: &str) -> String {
    normalize_rel(path)
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_default()
}

fn normal_module_dir(path: &str) -> String {
    let path = normalize_rel(path);
    let parent = parent_dir(&path);
    let file = path.rsplit('/').next().unwrap_or(&path);
    if file == "mod.rs" {
        parent
    } else {
        let stem = file.strip_suffix(".rs").unwrap_or(file);
        join_module_path(&parent, stem).unwrap_or(parent)
    }
}

fn join_module_path(base: &str, child: &str) -> Option<String> {
    let child = child.replace('\\', "/");
    if child.starts_with('/')
        || (child.len() >= 2
            && child.as_bytes()[0].is_ascii_alphabetic()
            && child.as_bytes()[1] == b':')
    {
        return None;
    }
    let mut parts = Vec::new();
    let base = base.replace('\\', "/");
    for part in base.split('/').chain(child.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    Some(parts.join("/"))
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
        Collection::WorkspaceRustTests => {
            let file = normalize_rel(file);
            packages.iter().any(|package| {
                package
                    .targets
                    .iter()
                    .any(|target| target.files.contains(&file))
            })
        }
    }
}

fn is_non_proof_surface(file: &str) -> bool {
    let normalized = file.replace('\\', "/");
    if matches!(
        normalized
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.')),
        Some((
            _,
            "md" | "markdown" | "rst" | "txt" | "json" | "toml" | "yaml" | "yml"
        ))
    ) {
        return true;
    }

    normalized.split('/').any(|part| {
        matches!(
            part,
            "doc"
                | "docs"
                | "documentation"
                | "example"
                | "examples"
                | "fixture"
                | "fixtures"
                | "generated"
                | "vendor"
        )
    })
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

pub(crate) fn index_class_files<T: TreeReader>(
    tree: &T,
    files: &[String],
) -> Result<(Vec<IndexedCitation>, Vec<String>), Vec<String>> {
    let mut citations = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        if is_non_proof_surface(file) {
            continue;
        }
        let text = match tree.read_text(file) {
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

    #[derive(Default)]
    struct MemoryTree {
        files: BTreeMap<String, String>,
    }

    impl MemoryTree {
        fn with(mut self, path: &str, text: &str) -> Self {
            self.files.insert(path.to_owned(), text.to_owned());
            self
        }
    }

    impl TreeReader for MemoryTree {
        fn tracked_files(&self) -> Result<Vec<String>, String> {
            Ok(self.files.keys().cloned().collect())
        }

        fn pathspec_files(&self, _pathspecs: &[String]) -> Result<Vec<String>, String> {
            self.tracked_files()
        }

        fn read_text(&self, rel: &str) -> Result<Option<String>, String> {
            Ok(self.files.get(rel).cloned())
        }
    }

    #[test]
    fn workspace_collection_follows_declared_modules_not_source_directory() {
        let tree = MemoryTree::default()
            .with(
                "Cargo.toml",
                "[workspace]\nmembers = [\"crate_a\", \"tests/central\"]\n",
            )
            .with(
                "crate_a/Cargo.toml",
                "[package]\nname = \"crate_a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .with(
                "crate_a/src/lib.rs",
                "mod included;\n#[path = \"../../path_import.rs\"]\nmod imported;\n",
            )
            .with(
                "crate_a/src/included.rs",
                "mod nested;\n#[path = \"path_sibling.rs\"]\nmod sibling;\n",
            )
            .with("crate_a/src/included/nested.rs", "")
            .with("crate_a/src/path_sibling.rs", "")
            .with("crate_a/src/included/path_sibling.rs", "")
            .with("crate_a/src/not_a_module.rs", "")
            .with("path_import.rs", "")
            .with(
                "tests/central/Cargo.toml",
                "[package]\nname = \"central\"\nversion = \"0.1.0\"\nedition = \"2021\"\nautotests = false\n[[test]]\nname = \"workspace\"\npath = \"tests/workspace.rs\"\n",
            )
            .with(
                "tests/central/tests/workspace.rs",
                "#[path = \"../../../central_import.rs\"]\nmod imported_test;\n",
            )
            .with("central_import.rs", "mod imported_child;\n")
            .with("imported_child.rs", "");
        let packages = workspace_packages(&tree).unwrap();
        let collected = Collection::WorkspaceRustTests;
        assert!(collection_contains(
            "crate_a/src/lib.rs",
            &collected,
            &packages
        ));
        assert!(collection_contains(
            "crate_a/src/included/nested.rs",
            &collected,
            &packages
        ));
        assert!(collection_contains(
            "crate_a/src/path_sibling.rs",
            &collected,
            &packages
        ));
        assert!(!collection_contains(
            "crate_a/src/included/path_sibling.rs",
            &collected,
            &packages
        ));
        assert!(!collection_contains(
            "crate_a/path_import.rs",
            &collected,
            &packages
        ));
        assert!(collection_contains("path_import.rs", &collected, &packages));
        assert!(collection_contains(
            "tests/central/tests/workspace.rs",
            &collected,
            &packages
        ));
        assert!(collection_contains(
            "central_import.rs",
            &collected,
            &packages
        ));
        assert!(collection_contains(
            "imported_child.rs",
            &collected,
            &packages
        ));
        assert!(!collection_contains(
            "crate_a/src/not_a_module.rs",
            &collected,
            &packages
        ));
    }
}
