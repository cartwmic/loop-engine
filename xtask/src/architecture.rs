use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::{Utf8Path, Utf8PathBuf};
use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, Package, TargetKind};
use walkdir::WalkDir;

const PRODUCT_CRATES: [&str; 3] = [
    "loop-engine-core",
    "loop-engine-integrations",
    "loop-engine-cli",
];

const NON_PRODUCT_WORKSPACE_MEMBERS: [&str; 1] = ["xtask"];

/// Allowed normal product-to-product dependency edges (dependent, dependency).
const ALLOWED_PRODUCT_EDGES: [(&str, &str); 3] = [
    ("loop-engine-integrations", "loop-engine-core"),
    ("loop-engine-cli", "loop-engine-core"),
    ("loop-engine-cli", "loop-engine-integrations"),
];

const FORBIDDEN_CATCH_ALL_MODULES: [&str; 2] = ["util", "common"];

const APPROVED_PROVIDER_CONSTRUCTION_PREFIXES: [&str; 1] = ["provider_process/"];
const APPROVED_PERSISTENCE_CONSTRUCTION_PREFIXES: [&str; 1] = ["persistence/"];
const APPROVED_CLI_COMPOSITION_FILES: [&str; 1] = ["composition.rs"];
const APPROVED_CLI_DISPATCH_FILES: [&str; 1] = ["dispatch.rs"];

const PROCESS_CONSTRUCTION_MARKERS: [&str; 8] = [
    "std::process",
    "std::{process",
    "process::Command",
    "Command::new",
    "Command as",
    "use std as",
    "std::{self as",
    "extern crate std as",
];

const SQLITE_CONSTRUCTION_MARKERS: [&str; 1] = ["rusqlite"];

const DISPATCH_BYPASS_MARKERS: [&str; 10] = [
    "loop_engine_core::operations",
    "loop_engine_core::{operations",
    "use loop_engine_core::operations",
    "use loop_engine_core as",
    "use ::loop_engine_core as",
    "extern crate loop_engine_core as",
    "loop_engine_core::{self as",
    "use loop_engine_core::*",
    "loop_engine_core::{*",
    "use ::loop_engine_core::*",
];

/// Verify product architecture for the workspace rooted at `manifest_path`.
///
/// When `manifest_path` is `None`, Cargo resolves the workspace from the current directory.
pub fn run(manifest_path: Option<&Path>) -> Result<()> {
    let metadata = load_metadata(manifest_path)?;
    check_product_crates(&metadata)?;
    check_product_dependency_edges(&metadata)?;
    check_sole_composition_root(&metadata)?;
    check_core_internal_boundaries(&metadata)?;
    check_forbidden_catch_all_modules(&metadata)?;
    check_construction_and_dispatch_bypass(&metadata)?;
    Ok(())
}

fn load_metadata(manifest_path: Option<&Path>) -> Result<Metadata> {
    let mut command = MetadataCommand::new();
    if let Some(path) = manifest_path {
        let utf8 = Utf8Path::from_path(path)
            .with_context(|| format!("manifest path is not valid UTF-8: {}", path.display()))?;
        command.manifest_path(utf8);
    }

    command
        .exec()
        .context("failed to load Cargo metadata for architecture check")
}

fn check_product_crates(metadata: &Metadata) -> Result<()> {
    let workspace_members = workspace_member_names(metadata)?;
    let product_members = workspace_members
        .iter()
        .filter(|name| !NON_PRODUCT_WORKSPACE_MEMBERS.contains(&name.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();

    let expected = PRODUCT_CRATES
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    if product_members == expected {
        return Ok(());
    }

    let mut violations = Vec::new();

    for missing in expected.difference(&product_members) {
        violations.push(format!(
            "missing required product crate workspace member `{missing}` (I22)"
        ));
    }

    for unexpected in product_members.difference(&expected) {
        violations.push(format!(
            "unexpected product crate workspace member `{unexpected}`; product code must use exactly `{}`, `{}`, and `{}` (I22)",
            PRODUCT_CRATES[0], PRODUCT_CRATES[1], PRODUCT_CRATES[2]
        ));
    }

    bail!(format_violations(
        "product crate workspace membership",
        violations
    ));
}

fn check_product_dependency_edges(metadata: &Metadata) -> Result<()> {
    let package_ids = metadata
        .packages
        .iter()
        .map(|package| (package.id.clone(), package.name.clone()))
        .collect::<HashMap<_, _>>();

    let product_names = PRODUCT_CRATES.iter().copied().collect::<HashSet<&str>>();

    let allowed_edges = ALLOWED_PRODUCT_EDGES
        .iter()
        .copied()
        .collect::<HashSet<(&str, &str)>>();

    let resolve = metadata
        .resolve
        .as_ref()
        .context("Cargo metadata did not include a resolved dependency graph")?;

    let mut violations = Vec::new();

    for product in PRODUCT_CRATES {
        let package = metadata
            .packages
            .iter()
            .find(|candidate| candidate.name == product)
            .with_context(|| format!("resolved metadata is missing product crate `{product}`"))?;

        for dependency in &package.dependencies {
            if !allowed_product_dependency(product, &dependency.name, dependency.kind) {
                violations.push(format!(
                    "forbidden declared dependency `{product}` -> `{}` ({:?}); dependency is outside the product crate allowlist (I22)",
                    dependency.name, dependency.kind
                ));
            }
        }

        let node = resolve
            .nodes
            .iter()
            .find(|node| node.id == package.id)
            .with_context(|| format!("resolved metadata is missing node for `{product}`"))?;

        for dependency in &node.deps {
            let dependency_name = package_ids.get(&dependency.pkg).with_context(|| {
                format!("resolved dependency for `{product}` has unknown package id")
            })?;

            if !product_names.contains(dependency_name.as_str()) {
                continue;
            }

            if !dependency
                .dep_kinds
                .iter()
                .any(|kind| matches!(kind.kind, DependencyKind::Normal))
            {
                continue;
            }

            let edge = (product, dependency_name.as_str());
            if allowed_edges.contains(&edge) {
                continue;
            }

            violations.push(format!(
                "forbidden product dependency `{dependent}` -> `{dependency}`; allowed edges are integrations -> core and cli -> {{core, integrations}} (I22)",
                dependent = edge.0,
                dependency = edge.1
            ));
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        bail!(format_violations(
            "product crate dependency edges",
            violations
        ));
    }
}

fn check_sole_composition_root(metadata: &Metadata) -> Result<()> {
    let mut violations = Vec::new();

    for product in PRODUCT_CRATES {
        let package = product_package(metadata, product)?;
        let has_bin = package
            .targets
            .iter()
            .any(|target| target.kind.contains(&TargetKind::Bin));

        match (product, has_bin) {
            ("loop-engine-cli", false) => {
                violations.push(format!(
                    "`{product}` must expose the production binary target as the sole composition root (I22)"
                ));
            }
            ("loop-engine-cli", true) => {}
            (_, true) => {
                violations.push(format!(
                    "`{product}` must not expose a binary target; only `loop-engine-cli` is the composition root (I22)"
                ));
            }
            _ => {}
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        bail!(format_violations("composition root", violations));
    }
}

fn check_core_internal_boundaries(metadata: &Metadata) -> Result<()> {
    let package = product_package(metadata, "loop-engine-core")?;
    let src_root = package_src_root(package)?;
    if !src_root.is_dir() {
        return Ok(());
    }

    let mut violations = Vec::new();

    for (relative_path, source) in rust_sources_under(&src_root)? {
        let layer = classify_core_layer(&relative_path);
        for (line_number, line) in source_lines(&source) {
            for marker in forbidden_core_import_markers(layer) {
                if line_contains_marker(line, marker) {
                    violations.push(format!(
                        "forbidden core internal dependency in `{relative_path}` line {line_number}: `{marker}` is not allowed in the `{}` layer (I23)",
                        layer_name(layer)
                    ));
                }
            }
        }
        for marker in forbidden_core_import_markers(layer) {
            let found_on_line =
                source_lines(&source).any(|(_, line)| line_contains_marker(line, marker));
            if source_contains_marker(&source, marker) && !found_on_line {
                violations.push(format!(
                    "forbidden multiline core internal dependency in `{relative_path}`: `{marker}` is not allowed in the `{}` layer (I23)",
                    layer_name(layer)
                ));
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        bail!(format_violations(
            "core internal dependency direction",
            violations
        ));
    }
}

fn check_forbidden_catch_all_modules(metadata: &Metadata) -> Result<()> {
    let mut violations = Vec::new();

    for product in PRODUCT_CRATES {
        let package = product_package(metadata, product)?;
        let src_root = package_src_root(package)?;
        if !src_root.is_dir() {
            continue;
        }

        for entry in fs::read_dir(src_root.as_std_path())
            .with_context(|| format!("failed to read `{}`", src_root))?
        {
            let entry =
                entry.with_context(|| format!("failed to read entry under `{src_root}`"))?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if FORBIDDEN_CATCH_ALL_MODULES.contains(&name.as_ref())
                && (entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                    || name.ends_with(".rs"))
            {
                violations.push(format!(
                    "forbidden catch-all module `{name}` in `{product}`; product crates must not use generic `util` or `common` modules"
                ));
            }
        }

        for entry in WalkDir::new(src_root.as_std_path())
            .min_depth(2)
            .into_iter()
            .filter_map(Result::ok)
        {
            let name = entry.file_name().to_string_lossy();
            let catch_all = if entry.file_type().is_dir() {
                FORBIDDEN_CATCH_ALL_MODULES.contains(&name.as_ref())
            } else if entry.path().extension().is_some_and(|ext| ext == "rs") {
                entry.path().file_stem().is_some_and(|stem| {
                    FORBIDDEN_CATCH_ALL_MODULES.contains(&stem.to_string_lossy().as_ref())
                })
            } else {
                false
            };
            if catch_all {
                let relative = entry
                    .path()
                    .strip_prefix(src_root.as_std_path())
                    .unwrap_or(entry.path());
                violations.push(format!(
                    "forbidden nested catch-all module `{}` in `{product}/src/{}`",
                    name,
                    relative.display()
                ));
            }
        }

        for (relative_path, source) in rust_sources_under(&src_root)? {
            for module in FORBIDDEN_CATCH_ALL_MODULES {
                if source_declares_module(&source, module) {
                    violations.push(format!(
                        "forbidden inline catch-all module `{module}` in `{product}/src/{relative_path}`"
                    ));
                }
            }
        }

        for crate_root in crate_root_sources(package)? {
            if !crate_root.is_file() {
                continue;
            }
            let source = fs::read_to_string(crate_root.as_std_path())
                .with_context(|| format!("failed to read catch-all crate root `{crate_root}`"))?;
            for (line_number, line) in source_lines(&source) {
                for module in FORBIDDEN_CATCH_ALL_MODULES {
                    let marker = format!("mod {module};");
                    if line_contains_marker(line, &marker) {
                        violations.push(format!(
                            "forbidden catch-all module declaration in `{crate_root}` line {line_number}: `{marker}`"
                        ));
                    }
                }
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        bail!(format_violations("forbidden catch-all modules", violations));
    }
}

fn check_construction_and_dispatch_bypass(metadata: &Metadata) -> Result<()> {
    let mut violations = Vec::new();

    for product in PRODUCT_CRATES {
        let package = product_package(metadata, product)?;
        let src_root = package_src_root(package)?;
        if !src_root.is_dir() {
            continue;
        }

        for (relative_path, source) in rust_sources_under(&src_root)? {
            if product != "loop-engine-core" && source_publicly_reexports_raw_integration(&source) {
                violations.push(format!(
                    "forbidden multiline raw integration re-export in `{product}/src/{relative_path}`; expose a wrapper capability instead"
                ));
            }
            for (line_number, line) in source_lines(&source) {
                if product != "loop-engine-core"
                    && (PROCESS_CONSTRUCTION_MARKERS
                        .iter()
                        .chain(SQLITE_CONSTRUCTION_MARKERS.iter()))
                    .any(|marker| {
                        line_contains_marker(line, marker)
                            && line_publicly_reexports_raw_integration(line)
                    })
                {
                    violations.push(format!(
                        "forbidden raw integration re-export in `{product}/src/{relative_path}` line {line_number}; expose a wrapper capability instead"
                    ));
                }

                if product == "loop-engine-core" {
                    for marker in PROCESS_CONSTRUCTION_MARKERS {
                        if line_contains_marker(line, marker) {
                            violations.push(format!(
                                "forbidden provider-process construction in `loop-engine-core/src/{relative_path}` line {line_number}: `{marker}` (core must not instantiate process integrations)"
                            ));
                        }
                    }
                    for marker in SQLITE_CONSTRUCTION_MARKERS {
                        if line_contains_marker(line, marker) {
                            violations.push(format!(
                                "forbidden persistence construction in `loop-engine-core/src/{relative_path}` line {line_number}: `{marker}` (core must not instantiate persistence integrations)"
                            ));
                        }
                    }
                }

                if product == "loop-engine-integrations" {
                    if !is_approved_provider_construction_path(&relative_path) {
                        for marker in PROCESS_CONSTRUCTION_MARKERS {
                            if line_contains_marker(line, marker) {
                                violations.push(format!(
                                    "forbidden provider-process construction bypass in `loop-engine-integrations/src/{relative_path}` line {line_number}: `{marker}` must stay under `provider_process/`"
                                ));
                            }
                        }
                    }
                    if !is_approved_persistence_construction_path(&relative_path) {
                        for marker in SQLITE_CONSTRUCTION_MARKERS {
                            if line_contains_marker(line, marker) {
                                violations.push(format!(
                                    "forbidden persistence construction bypass in `loop-engine-integrations/src/{relative_path}` line {line_number}: `{marker}` must stay under `persistence/`"
                                ));
                            }
                        }
                    }
                }

                if product == "loop-engine-cli" {
                    if !is_approved_cli_composition_path(&relative_path) {
                        for marker in PROCESS_CONSTRUCTION_MARKERS {
                            if line_contains_marker(line, marker) {
                                violations.push(format!(
                                    "forbidden provider-process construction bypass in `loop-engine-cli/src/{relative_path}` line {line_number}: `{marker}` must stay in `composition.rs`"
                                ));
                            }
                        }
                        for marker in SQLITE_CONSTRUCTION_MARKERS {
                            if line_contains_marker(line, marker) {
                                violations.push(format!(
                                    "forbidden persistence construction bypass in `loop-engine-cli/src/{relative_path}` line {line_number}: `{marker}` must stay in `composition.rs`"
                                ));
                            }
                        }
                    }
                    if !is_approved_cli_dispatch_path(&relative_path) {
                        for marker in DISPATCH_BYPASS_MARKERS {
                            if line_contains_marker(line, marker) {
                                violations.push(format!(
                                    "forbidden operation-dispatch bypass in `loop-engine-cli/src/{relative_path}` line {line_number}: `{marker}` must stay in `dispatch.rs`"
                                ));
                            }
                        }
                    }
                }
            }

            if product == "loop-engine-core" {
                record_multiline_bypass(
                    &mut violations,
                    &source,
                    &relative_path,
                    &PROCESS_CONSTRUCTION_MARKERS,
                    "provider-process construction",
                );
                record_multiline_bypass(
                    &mut violations,
                    &source,
                    &relative_path,
                    &SQLITE_CONSTRUCTION_MARKERS,
                    "persistence construction",
                );
            }
            if product == "loop-engine-integrations" {
                if !is_approved_provider_construction_path(&relative_path) {
                    record_multiline_bypass(
                        &mut violations,
                        &source,
                        &relative_path,
                        &PROCESS_CONSTRUCTION_MARKERS,
                        "provider-process construction bypass",
                    );
                }
                if !is_approved_persistence_construction_path(&relative_path) {
                    record_multiline_bypass(
                        &mut violations,
                        &source,
                        &relative_path,
                        &SQLITE_CONSTRUCTION_MARKERS,
                        "persistence construction bypass",
                    );
                }
            }
            if product == "loop-engine-cli" {
                if !is_approved_cli_composition_path(&relative_path) {
                    record_multiline_bypass(
                        &mut violations,
                        &source,
                        &relative_path,
                        &PROCESS_CONSTRUCTION_MARKERS,
                        "provider-process construction bypass",
                    );
                    record_multiline_bypass(
                        &mut violations,
                        &source,
                        &relative_path,
                        &SQLITE_CONSTRUCTION_MARKERS,
                        "persistence construction bypass",
                    );
                }
                if !is_approved_cli_dispatch_path(&relative_path) {
                    record_multiline_bypass(
                        &mut violations,
                        &source,
                        &relative_path,
                        &DISPATCH_BYPASS_MARKERS,
                        "operation-dispatch bypass",
                    );
                }
            }
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        bail!(format_violations(
            "integration construction and dispatch choke points",
            violations
        ));
    }
}

fn record_multiline_bypass(
    violations: &mut Vec<String>,
    source: &str,
    relative_path: &str,
    markers: &[&str],
    label: &str,
) {
    for marker in markers {
        let found_on_line =
            source_lines(source).any(|(_, line)| line_contains_marker(line, marker));
        if source_contains_marker(source, marker) && !found_on_line {
            violations.push(format!(
                "forbidden multiline {label} in `{relative_path}`: `{marker}` crosses lines"
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoreLayer {
    Root,
    Model,
    Capabilities,
    Operations,
}

fn allowed_product_dependency(product: &str, dependency: &str, kind: DependencyKind) -> bool {
    let allowed: &[&str] = match (product, kind) {
        ("loop-engine-core", DependencyKind::Normal) => {
            &["thiserror", "jiff", "uuid", "sha2", "petgraph"]
        }
        ("loop-engine-core", DependencyKind::Development) => &["proptest"],
        ("loop-engine-integrations", DependencyKind::Normal) => &[
            "loop-engine-core",
            "rusqlite",
            "rusqlite_migration",
            "serde",
            "serde_json",
            "thiserror",
            "jiff",
            "uuid",
            "sha2",
            "schemars",
            "tracing",
            "toml",
        ],
        ("loop-engine-integrations", DependencyKind::Development) => &["tempfile"],
        ("loop-engine-cli", DependencyKind::Normal) => &[
            "loop-engine-core",
            "loop-engine-integrations",
            "clap",
            "miette",
            "serde",
            "serde_json",
            "thiserror",
            "jiff",
            "tracing",
            "tracing-subscriber",
            "tracing-appender",
        ],
        ("loop-engine-cli", DependencyKind::Development) => &["assert_cmd", "tempfile"],
        _ => &[],
    };
    allowed.contains(&dependency)
}

fn classify_core_layer(relative_path: &str) -> CoreLayer {
    if relative_path.starts_with("model/")
        || relative_path == "model.rs"
        || relative_path.ends_with("/model.rs")
    {
        CoreLayer::Model
    } else if relative_path.starts_with("capabilities/")
        || relative_path == "capabilities.rs"
        || relative_path.ends_with("/capabilities.rs")
    {
        CoreLayer::Capabilities
    } else if relative_path.starts_with("operations/")
        || relative_path == "operations.rs"
        || relative_path.ends_with("/operations.rs")
    {
        CoreLayer::Operations
    } else {
        CoreLayer::Root
    }
}

fn layer_name(layer: CoreLayer) -> &'static str {
    match layer {
        CoreLayer::Root => "crate root",
        CoreLayer::Model => "model",
        CoreLayer::Capabilities => "capabilities",
        CoreLayer::Operations => "operations",
    }
}

fn forbidden_core_import_markers(layer: CoreLayer) -> &'static [&'static str] {
    match layer {
        CoreLayer::Model => &[
            "crate::capabilities",
            "crate::{capabilities",
            "crate::operations",
            "crate::{operations",
            "loop_engine_core::capabilities",
            "loop_engine_core::{capabilities",
            "loop_engine_core::operations",
            "loop_engine_core::{operations",
            "super::capabilities",
            "super::{capabilities",
            "super::operations",
            "super::{operations",
            "use crate as",
            "crate::{self as",
            "use crate::*",
            "crate::{*",
            "extern crate self as",
            "use super as",
            "super::{self as",
            "use super::*",
            "super::{*",
            "use loop_engine_core::*",
            "loop_engine_core::{*",
            "use ::loop_engine_core as",
            "use ::loop_engine_core::*",
            "extern crate loop_engine_core as",
        ],
        CoreLayer::Capabilities => &[
            "crate::operations",
            "crate::{operations",
            "loop_engine_core::operations",
            "loop_engine_core::{operations",
            "super::operations",
            "super::{operations",
            "use crate as",
            "crate::{self as",
            "use crate::*",
            "crate::{*",
            "extern crate self as",
            "use super as",
            "super::{self as",
            "use super::*",
            "super::{*",
            "use loop_engine_core::*",
            "loop_engine_core::{*",
            "use ::loop_engine_core as",
            "use ::loop_engine_core::*",
            "extern crate loop_engine_core as",
        ],
        CoreLayer::Root | CoreLayer::Operations => &[],
    }
}

fn is_approved_provider_construction_path(relative_path: &str) -> bool {
    APPROVED_PROVIDER_CONSTRUCTION_PREFIXES
        .iter()
        .any(|prefix| relative_path.starts_with(prefix))
}

fn is_approved_persistence_construction_path(relative_path: &str) -> bool {
    APPROVED_PERSISTENCE_CONSTRUCTION_PREFIXES
        .iter()
        .any(|prefix| relative_path.starts_with(prefix))
}

fn is_approved_cli_composition_path(relative_path: &str) -> bool {
    APPROVED_CLI_COMPOSITION_FILES.contains(&relative_path)
}

fn is_approved_cli_dispatch_path(relative_path: &str) -> bool {
    APPROVED_CLI_DISPATCH_FILES.contains(&relative_path)
}

fn product_package<'a>(metadata: &'a Metadata, name: &str) -> Result<&'a Package> {
    metadata
        .packages
        .iter()
        .find(|package| package.name == name)
        .with_context(|| format!("resolved metadata is missing product crate `{name}`"))
}

fn package_src_root(package: &Package) -> Result<Utf8PathBuf> {
    Ok(package
        .manifest_path
        .parent()
        .context("package manifest path has no parent directory")?
        .join("src"))
}

fn crate_root_sources(package: &Package) -> Result<Vec<Utf8PathBuf>> {
    let package_root = package
        .manifest_path
        .parent()
        .context("package manifest path has no parent directory")?;
    let mut roots = vec![package_root.join("src").join("lib.rs")];
    if package.name == "loop-engine-cli" {
        roots.push(package_root.join("src").join("main.rs"));
    }
    Ok(roots)
}

fn rust_sources_under(src_root: &Utf8Path) -> Result<Vec<(String, String)>> {
    let mut sources = Vec::new();

    for entry in WalkDir::new(src_root.as_std_path())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }

        let relative = path
            .strip_prefix(src_root.as_std_path())
            .with_context(|| format!("failed to relativize source path `{}`", path.display()))?;
        let relative_path = relative
            .to_str()
            .with_context(|| format!("source path is not valid UTF-8: {}", relative.display()))?
            .replace('\\', "/");
        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read source file `{}`", path.display()))?;
        sources.push((relative_path, source));
    }

    sources.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(sources)
}

fn source_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
}

fn source_declares_module(source: &str, module: &str) -> bool {
    let code = source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    let compact: String = code.chars().filter(|ch| !ch.is_whitespace()).collect();
    for prefix in [format!("mod{module}"), format!("pubmod{module}")] {
        let mut rest = compact.as_str();
        while let Some(index) = rest.find(&prefix) {
            let suffix = &rest[index + prefix.len()..];
            if suffix.starts_with(';') || suffix.starts_with('{') {
                return true;
            }
            rest = &suffix[1.min(suffix.len())..];
        }
    }
    false
}

fn source_publicly_reexports_raw_integration(source: &str) -> bool {
    let compact: String = source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    compact.contains("pubusestd::process")
        || compact.contains("pubusestd::{process")
        || compact.contains("pub(crate)usestd::process")
        || compact.contains("pub(crate)usestd::{process")
        || compact.contains("pubuserusqlite")
        || compact.contains("pub(crate)userusqlite")
        || compact.contains("pubtype")
            && (compact.contains("std::process") || compact.contains("rusqlite"))
}

fn line_publicly_reexports_raw_integration(line: &str) -> bool {
    let compact: String = line
        .split("//")
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    compact.contains("pubuse") || compact.contains("pub(crate)use") || compact.contains("pubtype")
}

fn line_contains_marker(line: &str, marker: &str) -> bool {
    let Some(code) = line.split("//").next() else {
        return false;
    };
    compact_contains_marker(code, marker)
}

fn source_contains_marker(source: &str, marker: &str) -> bool {
    let code = source
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n");
    compact_contains_marker(&code, marker)
}

fn compact_contains_marker(code: &str, marker: &str) -> bool {
    if code.contains(marker) {
        return true;
    }
    let compact_code: String = code.chars().filter(|ch| !ch.is_whitespace()).collect();
    let compact_marker: String = marker.chars().filter(|ch| !ch.is_whitespace()).collect();
    compact_code.contains(&compact_marker)
}

fn workspace_member_names(metadata: &Metadata) -> Result<BTreeSet<String>> {
    let names = metadata
        .workspace_members
        .iter()
        .map(|member_id| {
            metadata
                .packages
                .iter()
                .find(|package| package.id == *member_id)
                .map(|package| package.name.to_string())
                .with_context(|| format!("workspace member id `{member_id}` has no package record"))
        })
        .collect::<Result<BTreeSet<_>>>()?;

    Ok(names)
}

fn format_violations(section: &str, violations: Vec<String>) -> String {
    let mut message = format!("architecture check failed: {section}");
    for violation in violations {
        message.push_str("\n- ");
        message.push_str(&violation);
    }
    message
}

/// Workspace manifest path for the repository that contains `xtask`.
pub fn default_workspace_manifest() -> Utf8PathBuf {
    Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_manifest(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/architecture")
            .join(name)
            .join("Cargo.toml")
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).expect("create fixture destination");
        for entry in fs::read_dir(source).expect("read fixture") {
            let entry = entry.expect("fixture entry");
            let target = destination.join(entry.file_name());
            if entry.file_type().expect("fixture type").is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), target).expect("copy fixture file");
            }
        }
    }

    #[test]
    fn alias_and_grouped_import_markers_cannot_bypass_choke_points() {
        for source in [
            "use std :: { process :: Command as C };",
            "let constructor = std :: process :: Command :: new;",
            "let constructor = Command::new;",
            "use rusqlite::{Connection as C};",
            "use rusqlite as db;",
            "use loop_engine_core::{operations as ops};",
            "use loop_engine_core as core;",
            "use ::loop_engine_core as core;",
            "extern crate loop_engine_core as core;",
            "use loop_engine_core::*;",
            "use std as standard_library;",
        ] {
            let detected = PROCESS_CONSTRUCTION_MARKERS
                .iter()
                .chain(SQLITE_CONSTRUCTION_MARKERS.iter())
                .chain(DISPATCH_BYPASS_MARKERS.iter())
                .any(|marker| line_contains_marker(source, marker));
            assert!(detected, "alias bypass was not detected: {source}");
        }
        assert!(source_contains_marker(
            "use loop_engine_core::{\n    operations as ops,\n};",
            "loop_engine_core::{operations"
        ));
        assert!(source_contains_marker(
            "use crate::{\n    operations as ops,\n};",
            "crate::{operations"
        ));
        assert!(source_contains_marker(
            "use crate::{\n    self as domain,\n};",
            "crate::{self as"
        ));
        assert!(source_publicly_reexports_raw_integration(
            "pub use std::{\n    process::Command,\n};"
        ));
        assert!(source_declares_module(
            "mod nested { pub mod common {} }",
            "common"
        ));
        for source in [
            "use crate::*;",
            "use super::*;",
            "use ::loop_engine_core::*;",
        ] {
            assert!(
                forbidden_core_import_markers(CoreLayer::Model)
                    .iter()
                    .any(|marker| source_contains_marker(source, marker)),
                "core glob bypass was not detected: {source}"
            );
        }
    }

    #[test]
    fn product_dependency_allowlists_reject_outer_leaks() {
        assert!(allowed_product_dependency(
            "loop-engine-core",
            "thiserror",
            DependencyKind::Normal
        ));
        assert!(!allowed_product_dependency(
            "loop-engine-core",
            "serde",
            DependencyKind::Normal
        ));
        assert!(!allowed_product_dependency(
            "loop-engine-core",
            "thiserror",
            DependencyKind::Build
        ));
    }

    #[test]
    fn current_repository_passes() {
        run(Some(default_workspace_manifest().as_std_path()))
            .expect("current repository should satisfy product architecture");
    }

    #[test]
    fn allowed_fixture_passes() {
        run(Some(&fixture_manifest("allowed")))
            .expect("allowed fixture should satisfy product architecture");
    }

    #[test]
    fn forbidden_edge_fixture_fails() {
        let error = run(Some(&fixture_manifest("forbidden-edge")))
            .expect_err("forbidden-edge fixture should violate product crate architecture");

        let message = error.to_string();
        assert!(
            message.contains("loop-engine-core")
                && message.contains("loop-engine-integrations")
                && message.contains("forbidden product dependency"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn bypass_fixture_fails_on_process_construction() {
        let error = run(Some(&fixture_manifest("bypass")))
            .expect_err("bypass fixture should violate construction choke points");

        let message = error.to_string();
        assert!(
            message.contains("provider-process construction")
                || message.contains("persistence construction"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn reversed_core_fixture_fails_on_internal_direction() {
        let error = run(Some(&fixture_manifest("reversed-core")))
            .expect_err("reversed-core fixture should violate core internal direction");

        let message = error.to_string();
        assert!(
            message.contains("core internal dependency direction")
                && message.contains("operations"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn nested_catch_all_module_is_rejected() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let source = fixture_manifest("allowed")
            .parent()
            .expect("fixture parent")
            .to_path_buf();
        copy_tree(&source, temp.path());
        let nested = temp.path().join("crates/loop-engine-core/src/model/common");
        fs::create_dir_all(&nested).expect("nested common");
        fs::write(nested.join("mod.rs"), "// forbidden nested catch-all\n").expect("nested module");

        let error = run(Some(&temp.path().join("Cargo.toml")))
            .expect_err("nested catch-all must violate policy");
        assert!(error.to_string().contains("nested catch-all"));
    }

    #[test]
    fn catch_all_fixture_fails_on_generic_module() {
        let error = run(Some(&fixture_manifest("catch-all")))
            .expect_err("catch-all fixture should violate catch-all module policy");

        let message = error.to_string();
        assert!(
            message.contains("catch-all"),
            "unexpected error message: {message}"
        );
    }
}
