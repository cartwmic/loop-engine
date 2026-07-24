use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const REQUIRED_COLLECTORS: &[&str] = &["core", "driver", "route", "e2e", "trace", "facet"];
const CATALOG_COLLECTORS: &[&str] = &["driver", "route"];
const EVIDENCE_COLLECTORS: &[&str] = &["e2e", "trace"];
const UNIVERSAL_FACET: &str = "Valid path through production CLI, runtime operation-ID proof, correlated trace file, request/outcome payloads, and start/finish envelope";
const FACET_NAMES: &[&str] = &[
    UNIVERSAL_FACET,
    "Run-state or run-journal mutation",
    "Successful creation",
    "Rejected/error creation",
    "Provider-catalog mutation",
    "Rejectable provider-catalog mutation",
    "Rejectable run mutation after run lookup",
    "Provider invoking",
    "Gate driven",
    "Read",
    "Lifecycle family",
    "Compatibility sensitive",
    "Provider-free under missing provider",
    "Journal required",
    "Trace provider boundary",
    "Trace persistence boundary",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageMode {
    Baseline,
    Candidate,
    Exposed,
    Final,
}

impl CoverageMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "candidate" => Ok(Self::Candidate),
            "exposed" => Ok(Self::Exposed),
            "final" => Ok(Self::Final),
            _ => bail!("unknown operation-coverage mode `{value}`"),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CoverageCollectors {
    sets: BTreeMap<&'static str, BTreeSet<String>>,
}

impl CoverageCollectors {
    pub fn register<I, S>(&mut self, name: &'static str, values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        if self.sets.contains_key(name) {
            bail!("duplicate operation collector `{name}`");
        }
        let values = values.into_iter().map(Into::into).collect::<Vec<_>>();
        let set = values.iter().cloned().collect::<BTreeSet<_>>();
        if set.len() != values.len() {
            bail!("collector `{name}` contains duplicate operation IDs");
        }
        self.sets.insert(name, set);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&BTreeSet<String>> {
        self.sets.get(name)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CoverageReport {
    pub mode: CoverageMode,
    pub planned: BTreeSet<String>,
    pub collectors: CoverageCollectors,
}

impl CoverageReport {
    pub fn summary(&self) -> String {
        let mode = match self.mode {
            CoverageMode::Baseline => "baseline",
            CoverageMode::Candidate => "candidate",
            CoverageMode::Exposed => "exposed",
            CoverageMode::Final => "final",
        };
        let mut parts = vec![format!("mode={mode}")];
        for name in REQUIRED_COLLECTORS {
            let count = self.collectors.get(name).map_or(0, BTreeSet::len);
            parts.push(format!("{name}={count}"));
        }
        parts.push(format!("planned={}", self.planned.len()));
        parts.join(" ")
    }
}

pub fn verify(
    mode: CoverageMode,
    allow_open: &BTreeSet<String>,
    planned: &BTreeSet<String>,
    collectors: &CoverageCollectors,
) -> Result<()> {
    if planned.len() != 21 {
        bail!("frozen operation catalog must contain exactly 21 IDs");
    }
    if !allow_open.is_subset(planned) {
        bail!("open operation IDs must belong to frozen catalog");
    }
    for (name, values) in &collectors.sets {
        if !values.is_subset(planned) {
            bail!("collector `{name}` contains IDs outside frozen catalog");
        }
    }
    for name in REQUIRED_COLLECTORS {
        if !collectors.sets.contains_key(name) {
            bail!("operation coverage has no registered `{name}` collector");
        }
    }
    let core = collectors
        .sets
        .get("core")
        .expect("core collector checked above");
    let driver = collectors
        .sets
        .get("driver")
        .expect("driver collector checked above");
    let route = collectors
        .sets
        .get("route")
        .expect("route collector checked above");
    let facet = collectors
        .sets
        .get("facet")
        .expect("facet collector checked above");

    if mode == CoverageMode::Baseline {
        if let Some((name, values)) = collectors
            .sets
            .iter()
            .find(|(_, values)| !values.is_empty())
        {
            bail!("baseline collector `{name}` is not empty: {values:?}");
        }
        if !allow_open.is_empty() {
            bail!("baseline mode does not accept open operation IDs");
        }
        return Ok(());
    }

    if !route.is_subset(driver) {
        bail!(
            "reachable route IDs must be driver-supported: `route`={route:?}, `driver`={driver:?}"
        );
    }

    for name in CATALOG_COLLECTORS {
        let values = collectors
            .sets
            .get(name)
            .expect("catalog collector checked above");
        if core != values {
            bail!("operation collector mismatch: `core`={core:?}, `{name}`={values:?}");
        }
    }

    match mode {
        CoverageMode::Candidate => {
            let open_facets = core.difference(facet).cloned().collect::<BTreeSet<_>>();
            if !open_facets.is_subset(allow_open) {
                bail!(
                    "open facet IDs must be declared in candidate mode: missing={open_facets:?}, allow_open={allow_open:?}, facet={facet:?}"
                );
            }
            if !facet.is_subset(core) {
                bail!(
                    "facet collector must not exceed core exposure: `core`={core:?}, `facet`={facet:?}"
                );
            }
        }
        CoverageMode::Exposed | CoverageMode::Final => {
            for name in EVIDENCE_COLLECTORS {
                let values = collectors
                    .sets
                    .get(name)
                    .expect("evidence collector checked above");
                if core != values {
                    bail!("operation collector mismatch: `core`={core:?}, `{name}`={values:?}");
                }
            }
            if core != facet {
                bail!("operation collector mismatch: `core`={core:?}, `facet`={facet:?}");
            }
        }
        CoverageMode::Baseline => unreachable!("baseline handled above"),
    }

    if mode != CoverageMode::Candidate && !allow_open.is_empty() {
        bail!("open operation IDs are permitted only in candidate mode");
    }
    if mode == CoverageMode::Final && core.len() != 21 {
        bail!("final operation set must contain exactly 21 IDs");
    }
    Ok(())
}

pub fn run(mode: CoverageMode, allow_open: &str) -> Result<()> {
    let root = locate_repository_root()?;
    run_at(&root, mode, allow_open)
}

pub fn run_at(root: &Path, mode: CoverageMode, allow_open: &str) -> Result<()> {
    let allow_open = parse_allow_open(allow_open);
    let report = collect_report(root, mode)?;
    verify(mode, &allow_open, &report.planned, &report.collectors)?;
    Ok(())
}

pub fn collect_report(root: &Path, mode: CoverageMode) -> Result<CoverageReport> {
    let planned = read_planned_catalog(root)?;
    let collectors = read_collectors(root)?;
    Ok(CoverageReport {
        mode,
        planned,
        collectors,
    })
}

fn parse_allow_open(allow_open: &str) -> BTreeSet<String> {
    allow_open
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn locate_repository_root() -> Result<PathBuf> {
    let mut root = std::env::current_dir()?;
    while !root
        .join("crates/loop-engine-core/src/operations/catalog.rs")
        .is_file()
    {
        if !root.pop() {
            bail!("could not locate repository root for operation coverage");
        }
    }
    Ok(root)
}

/// Read the frozen final operation catalog from the core source of truth.
pub fn final_operation_ids_at(root: &Path) -> Result<BTreeSet<String>> {
    read_planned_catalog(root)
}

fn read_planned_catalog(root: &Path) -> Result<BTreeSet<String>> {
    let source =
        fs::read_to_string(root.join("crates/loop-engine-core/src/operations/catalog.rs"))?;
    Ok(collect_array(
        &source,
        "pub const PLANNED_OPERATION_IDS: &[&str] = &[",
        "planned operation catalog",
    )?
    .into_iter()
    .collect())
}

fn read_collectors(root: &Path) -> Result<CoverageCollectors> {
    let core_source =
        fs::read_to_string(root.join("crates/loop-engine-core/src/operations/catalog.rs"))?;
    let driver_source =
        fs::read_to_string(root.join("crates/loop-engine-cli/src/driver_catalog.rs"))?;
    let mut collectors = CoverageCollectors::default();
    collectors.register(
        "core",
        collect_array(
            &core_source,
            "pub const EXPOSED_OPERATION_IDS: &[&str] = &[",
            "core operation collector",
        )?,
    )?;
    collectors.register(
        "driver",
        collect_array(
            &driver_source,
            "pub const DRIVER_OPERATION_IDS: &[&str] = &[",
            "driver operation collector",
        )?,
    )?;
    collectors.register(
        "route",
        collect_array(
            &driver_source,
            "pub const REACHABLE_ROUTE_OPERATION_IDS: &[&str] = &[",
            "reachable route operation collector",
        )?,
    )?;
    collectors.register(
        "e2e",
        collect_array(
            &driver_source,
            "pub const E2E_OPERATION_IDS: &[&str] = &[",
            "e2e operation collector",
        )?,
    )?;
    collectors.register(
        "trace",
        collect_array(
            &driver_source,
            "pub const TRACE_OPERATION_IDS: &[&str] = &[",
            "trace operation collector",
        )?,
    )?;
    let declared_facets = collect_array(
        &driver_source,
        "pub const FACET_OPERATION_IDS: &[&str] = &[",
        "facet operation collector",
    )?;
    let facet_manifests = read_facet_manifests(root, &declared_facets)?;
    collectors.register("facet", facet_manifests)?;
    Ok(collectors)
}

fn read_facet_manifests(root: &Path, declared: &[String]) -> Result<Vec<String>> {
    let directory = root.join("quality/facets/v1");
    let declared = declared.iter().cloned().collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json")
            || path.file_name().and_then(|value| value.to_str()) == Some("schema.json")
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                anyhow::anyhow!("facet manifest path is not valid UTF-8: {}", path.display())
            })?;
        validate_facet_manifest(root, &path, stem)?;
        if !observed.insert(stem.to_owned()) {
            bail!("duplicate facet manifest for operation `{stem}`");
        }
    }
    if observed != declared {
        bail!(
            "facet manifest inventory differs from declared facet catalog: declared={declared:?}, manifests={observed:?}"
        );
    }
    Ok(observed.into_iter().collect())
}

fn validate_facet_manifest(root: &Path, path: &Path, expected_operation: &str) -> Result<()> {
    let raw = fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid facet manifest JSON: {}", path.display()))?;
    let object = value.as_object().ok_or_else(|| {
        anyhow::anyhow!("facet manifest root must be an object: {}", path.display())
    })?;
    if object
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        bail!(
            "facet manifest schema_version must be 1: {}",
            path.display()
        );
    }
    if object
        .get("operation_id")
        .and_then(serde_json::Value::as_str)
        != Some(expected_operation)
    {
        bail!(
            "facet manifest operation_id must match filename `{expected_operation}`: {}",
            path.display()
        );
    }
    let facets = object
        .get("facets")
        .and_then(serde_json::Value::as_array)
        .filter(|facets| !facets.is_empty())
        .ok_or_else(|| anyhow::anyhow!("facet manifest must contain facets: {}", path.display()))?;
    let mut names = BTreeSet::new();
    let mut universal = 0_usize;
    for facet in facets {
        let facet = facet
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("facet row must be an object: {}", path.display()))?;
        let name = facet
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("facet row has no name: {}", path.display()))?;
        if !FACET_NAMES.contains(&name) {
            bail!("unknown facet name `{name}`: {}", path.display());
        }
        if !names.insert(name) {
            bail!("duplicate facet name `{name}`: {}", path.display());
        }
        universal += usize::from(name == UNIVERSAL_FACET);
        if facet.get("status").and_then(serde_json::Value::as_str) != Some("closed") {
            bail!("facet `{name}` is not closed: {}", path.display());
        }
        let evidence = facet
            .get("evidence")
            .and_then(serde_json::Value::as_array)
            .filter(|evidence| !evidence.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "closed facet `{name}` has no valid evidence: {}",
                    path.display()
                )
            })?;
        for item in evidence {
            let reference = item
                .as_str()
                .filter(|reference| !reference.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "closed facet `{name}` has no valid evidence: {}",
                        path.display()
                    )
                })?;
            validate_evidence_reference(root, path, name, reference)?;
        }
    }
    if universal != 1 {
        bail!(
            "facet manifest must contain exactly one universal valid-path row: {}",
            path.display()
        );
    }
    let required = required_facets(expected_operation)?
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if names != required {
        bail!(
            "facet manifest rows differ from required operation facets: operation={expected_operation}, required={required:?}, observed={names:?}"
        );
    }
    Ok(())
}

fn required_facets(operation: &str) -> Result<&'static [&'static str]> {
    let facets = match operation {
        "provider.add" => &[
            UNIVERSAL_FACET,
            "Provider-catalog mutation",
            "Rejectable provider-catalog mutation",
            "Trace persistence boundary",
        ][..],
        "provider.check" => &[
            UNIVERSAL_FACET,
            "Provider invoking",
            "Read",
            "Compatibility sensitive",
            "Trace provider boundary",
            "Trace persistence boundary",
        ],
        "provider.update" | "provider.rename" | "provider.restore" => &[
            UNIVERSAL_FACET,
            "Provider-catalog mutation",
            "Rejectable provider-catalog mutation",
            "Trace persistence boundary",
        ],
        "provider.disable" => &[
            UNIVERSAL_FACET,
            "Provider-catalog mutation",
            "Rejectable provider-catalog mutation",
            "Trace persistence boundary",
        ],
        "provider.list" => &[UNIVERSAL_FACET, "Read", "Trace persistence boundary"],
        "run.create" => &[
            UNIVERSAL_FACET,
            "Successful creation",
            "Rejected/error creation",
            "Provider invoking",
            "Trace provider boundary",
            "Trace persistence boundary",
        ],
        "run.graph" => &[
            UNIVERSAL_FACET,
            "Read",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.evidence.add" => &[
            UNIVERSAL_FACET,
            "Run-state or run-journal mutation",
            "Rejectable run mutation after run lookup",
            "Lifecycle family",
            "Journal required",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.evidence.list" => &[
            UNIVERSAL_FACET,
            "Read",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.annotate" => &[
            UNIVERSAL_FACET,
            "Run-state or run-journal mutation",
            "Rejectable run mutation after run lookup",
            "Lifecycle family",
            "Journal required",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.label" => &[
            UNIVERSAL_FACET,
            "Run-state or run-journal mutation",
            "Rejectable run mutation after run lookup",
            "Lifecycle family",
            "Journal required",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.history" => &[
            UNIVERSAL_FACET,
            "Read",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.guidance" => &[
            UNIVERSAL_FACET,
            "Run-state or run-journal mutation",
            "Rejectable run mutation after run lookup",
            "Provider invoking",
            "Lifecycle family",
            "Compatibility sensitive",
            "Journal required",
            "Trace provider boundary",
            "Trace persistence boundary",
        ],
        "run.compatibility" => &[
            UNIVERSAL_FACET,
            "Provider invoking",
            "Read",
            "Lifecycle family",
            "Compatibility sensitive",
            "Journal required",
            "Trace provider boundary",
            "Trace persistence boundary",
        ],
        "run.export" => &[
            UNIVERSAL_FACET,
            "Read",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.list" => &[
            UNIVERSAL_FACET,
            "Read",
            "Lifecycle family",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.request" => &[
            UNIVERSAL_FACET,
            "Run-state or run-journal mutation",
            "Rejectable run mutation after run lookup",
            "Provider invoking",
            "Gate driven",
            "Lifecycle family",
            "Compatibility sensitive",
            "Provider-free under missing provider",
            "Journal required",
            "Trace provider boundary",
            "Trace persistence boundary",
        ],
        "run.show" => &[
            UNIVERSAL_FACET,
            "Read",
            "Lifecycle family",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        "run.terminate" => &[
            UNIVERSAL_FACET,
            "Run-state or run-journal mutation",
            "Rejectable run mutation after run lookup",
            "Lifecycle family",
            "Journal required",
            "Provider-free under missing provider",
            "Trace persistence boundary",
        ],
        _ => bail!("no required facet catalog for operation `{operation}`"),
    };
    Ok(facets)
}

fn validate_evidence_reference(
    root: &Path,
    manifest: &Path,
    facet_name: &str,
    reference: &str,
) -> Result<()> {
    let trace_reference = reference.strip_prefix("trace:");
    if facet_name.starts_with("Trace ") != trace_reference.is_some() {
        bail!(
            "facet `{facet_name}` has mismatched trace evidence `{reference}`: {}",
            manifest.display()
        );
    }
    let reference = trace_reference.unwrap_or(reference);
    let (kind, target) = reference.split_once(':').ok_or_else(|| {
        anyhow::anyhow!(
            "invalid facet evidence reference `{reference}`: {}",
            manifest.display()
        )
    })?;
    let (module, test) = target.split_once("::").ok_or_else(|| {
        anyhow::anyhow!(
            "invalid facet evidence target `{target}`: {}",
            manifest.display()
        )
    })?;
    if ![module, test].into_iter().all(|identifier| {
        !identifier.is_empty()
            && identifier
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }) {
        bail!(
            "invalid facet evidence identifier `{target}`: {}",
            manifest.display()
        );
    }
    let source_path = match kind {
        "e2e" => registered_e2e_source(root, module)?,
        "catalog" => root
            .join("crates/loop-engine-cli/tests")
            .join(format!("{module}.rs")),
        _ => bail!(
            "unknown facet evidence kind `{kind}`: {}",
            manifest.display()
        ),
    };
    let source = fs::read_to_string(&source_path).with_context(|| {
        format!(
            "facet evidence source does not exist for `{reference}`: {}",
            source_path.display()
        )
    })?;
    if !contains_test_function(&source, test) {
        bail!(
            "facet evidence test `{reference}` does not exist in {}",
            source_path.display()
        );
    }
    Ok(())
}

fn attributes_disabled(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let path = attribute.path();
        path.is_ident("cfg") || path.is_ident("cfg_attr") || path.is_ident("ignore")
    })
}

fn registered_e2e_source(root: &Path, module: &str) -> Result<PathBuf> {
    #[derive(Default)]
    struct Registrations {
        named: usize,
        enabled: usize,
        valid: usize,
    }

    fn path_value(attributes: &[syn::Attribute]) -> Option<String> {
        attributes.iter().find_map(|attribute| {
            let syn::Meta::NameValue(name_value) = &attribute.meta else {
                return None;
            };
            if !name_value.path.is_ident("path") {
                return None;
            }
            let syn::Expr::Lit(expression) = &name_value.value else {
                return None;
            };
            let syn::Lit::Str(value) = &expression.lit else {
                return None;
            };
            Some(value.value()).filter(|value| !value.is_empty())
        })
    }

    fn scan(
        items: &[syn::Item],
        module_name: &str,
        expected_path: &str,
        parent_disabled: bool,
        top_level: bool,
        registrations: &mut Registrations,
    ) {
        for item in items {
            let syn::Item::Mod(item_module) = item else {
                continue;
            };
            let disabled = parent_disabled || attributes_disabled(&item_module.attrs);
            if item_module.ident == module_name {
                registrations.named += 1;
                if !disabled {
                    registrations.enabled += 1;
                    if top_level
                        && item_module.content.is_none()
                        && path_value(&item_module.attrs).as_deref() == Some(expected_path)
                    {
                        registrations.valid += 1;
                    }
                }
            }
            if let Some((_, nested)) = &item_module.content {
                scan(
                    nested,
                    module_name,
                    expected_path,
                    disabled,
                    false,
                    registrations,
                );
            }
        }
    }

    let test_directory = root.join("crates/loop-engine-cli/tests");
    let harness_path = test_directory.join("e2e.rs");
    let harness = fs::read_to_string(&harness_path).with_context(|| {
        format!(
            "E2E test harness does not exist: {}",
            harness_path.display()
        )
    })?;
    let file = syn::parse_file(&harness).with_context(|| {
        format!(
            "E2E test harness is not valid Rust: {}",
            harness_path.display()
        )
    })?;
    let expected_path = format!("e2e/{module}.rs");
    let mut registrations = Registrations::default();
    scan(
        &file.items,
        module,
        &expected_path,
        attributes_disabled(&file.attrs),
        true,
        &mut registrations,
    );
    if registrations.enabled == 0 {
        let reason = if registrations.named == 0 {
            "is not registered"
        } else {
            "is conditionally disabled"
        };
        bail!(
            "facet evidence module `{module}` {reason} in {}",
            harness_path.display()
        );
    }
    if registrations.enabled != 1 || registrations.valid != 1 {
        bail!(
            "facet evidence module `{module}` must have exactly one enabled top-level out-of-line registration using #[path = \"{expected_path}\"] in {}",
            harness_path.display()
        );
    }
    Ok(test_directory.join("e2e").join(format!("{module}.rs")))
}

fn contains_test_function(source: &str, test: &str) -> bool {
    fn find(items: &[syn::Item], test: &str, parent_disabled: bool) -> bool {
        items.iter().any(|item| match item {
            syn::Item::Fn(function) => {
                function.sig.ident == test
                    && !parent_disabled
                    && !attributes_disabled(&function.attrs)
                    && function
                        .attrs
                        .iter()
                        .any(|attribute| attribute.path().is_ident("test"))
            }
            syn::Item::Mod(module) => module.content.as_ref().is_some_and(|(_, items)| {
                find(
                    items,
                    test,
                    parent_disabled || attributes_disabled(&module.attrs),
                )
            }),
            _ => false,
        })
    }

    syn::parse_file(source)
        .is_ok_and(|file| !attributes_disabled(&file.attrs) && find(&file.items, test, false))
}

fn collect_array(source: &str, marker: &str, label: &str) -> Result<Vec<String>> {
    let start = source
        .find(marker)
        .ok_or_else(|| anyhow::anyhow!("{label} marker is missing"))?
        + marker.len();
    let body = source[start..]
        .split_once("];")
        .ok_or_else(|| anyhow::anyhow!("{label} is unterminated"))?
        .0;
    body.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_owned)
                .ok_or_else(|| anyhow::anyhow!("invalid {label} ID literal `{value}`"))
        })
        .collect::<Result<Vec<_>>>()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use serde_json::json;

    use super::{
        CoverageCollectors, CoverageMode, REQUIRED_COLLECTORS, UNIVERSAL_FACET,
        read_facet_manifests, required_facets, verify,
    };

    fn planned() -> BTreeSet<String> {
        std::iter::once("run.show".to_owned())
            .chain((1..21).map(|index| format!("operation.{index}")))
            .collect()
    }

    fn register_all<I, S>(collectors: &mut CoverageCollectors, values: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let values: Vec<String> = values
            .into_iter()
            .map(|value| value.as_ref().to_owned())
            .collect();
        for name in REQUIRED_COLLECTORS {
            collectors.register(name, values.iter().cloned())?;
        }
        Ok(())
    }

    #[test]
    fn baseline_accepts_empty_and_rejects_nonempty() {
        let mut empty = CoverageCollectors::default();
        register_all(&mut empty, std::iter::empty::<&str>()).unwrap();
        verify(CoverageMode::Baseline, &BTreeSet::new(), &planned(), &empty).unwrap();

        let mut nonempty = CoverageCollectors::default();
        register_all(&mut nonempty, std::iter::empty::<&str>()).unwrap();
        nonempty
            .sets
            .get_mut("core")
            .unwrap()
            .insert("run.show".to_owned());
        assert!(
            verify(
                CoverageMode::Baseline,
                &BTreeSet::new(),
                &planned(),
                &nonempty
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_missing_and_unknown_id_canaries_fail() {
        let mut duplicate = CoverageCollectors::default();
        assert!(
            duplicate
                .register("core", ["run.show", "run.show"])
                .is_err()
        );
        assert!(
            duplicate
                .register("driver", ["run.show", "run.show"])
                .is_err()
        );

        let mut missing_core = CoverageCollectors::default();
        missing_core
            .register("driver", Vec::<String>::new())
            .unwrap();
        assert!(
            verify(
                CoverageMode::Baseline,
                &BTreeSet::new(),
                &planned(),
                &missing_core
            )
            .is_err()
        );

        let mut missing_driver = CoverageCollectors::default();
        missing_driver
            .register("core", Vec::<String>::new())
            .unwrap();
        missing_driver
            .register("route", Vec::<String>::new())
            .unwrap();
        missing_driver
            .register("e2e", Vec::<String>::new())
            .unwrap();
        missing_driver
            .register("trace", Vec::<String>::new())
            .unwrap();
        missing_driver
            .register("facet", Vec::<String>::new())
            .unwrap();
        assert!(
            verify(
                CoverageMode::Baseline,
                &BTreeSet::new(),
                &planned(),
                &missing_driver
            )
            .is_err()
        );

        let mut missing_route = CoverageCollectors::default();
        missing_route
            .register("core", Vec::<String>::new())
            .unwrap();
        missing_route
            .register("driver", Vec::<String>::new())
            .unwrap();
        missing_route.register("e2e", Vec::<String>::new()).unwrap();
        missing_route
            .register("trace", Vec::<String>::new())
            .unwrap();
        missing_route
            .register("facet", Vec::<String>::new())
            .unwrap();
        assert!(
            verify(
                CoverageMode::Baseline,
                &BTreeSet::new(),
                &planned(),
                &missing_route
            )
            .is_err()
        );

        let mut empty = CoverageCollectors::default();
        register_all(&mut empty, std::iter::empty::<&str>()).unwrap();
        assert!(
            verify(
                CoverageMode::Candidate,
                &BTreeSet::from(["totally.not-an-operation".to_owned()]),
                &planned(),
                &empty
            )
            .is_err()
        );
    }

    #[test]
    fn missing_driver_and_route_canaries_fail_in_exposed_mode() {
        let mut missing_driver = CoverageCollectors::default();
        register_all(&mut missing_driver, ["run.show"]).unwrap();
        missing_driver.sets.get_mut("driver").unwrap().clear();
        assert!(
            verify(
                CoverageMode::Exposed,
                &BTreeSet::new(),
                &planned(),
                &missing_driver
            )
            .is_err()
        );

        let mut missing_route = CoverageCollectors::default();
        register_all(&mut missing_route, ["run.show"]).unwrap();
        missing_route.sets.get_mut("route").unwrap().clear();
        assert!(
            verify(
                CoverageMode::Exposed,
                &BTreeSet::new(),
                &planned(),
                &missing_route
            )
            .is_err()
        );

        let mut route_without_driver = CoverageCollectors::default();
        register_all(&mut route_without_driver, ["run.show"]).unwrap();
        route_without_driver.sets.get_mut("driver").unwrap().clear();
        route_without_driver
            .sets
            .get_mut("route")
            .unwrap()
            .insert("run.show".to_owned());
        assert!(
            verify(
                CoverageMode::Exposed,
                &BTreeSet::new(),
                &planned(),
                &route_without_driver
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_requires_catalog_equality_and_declared_open_facets() {
        let mut closed = CoverageCollectors::default();
        register_all(&mut closed, ["run.show"]).unwrap();
        verify(
            CoverageMode::Candidate,
            &BTreeSet::new(),
            &planned(),
            &closed,
        )
        .unwrap();

        let mut open = CoverageCollectors::default();
        register_all(&mut open, ["run.show", "operation.1"]).unwrap();
        open.sets
            .get_mut("facet")
            .unwrap()
            .retain(|value| value != "operation.1");
        verify(
            CoverageMode::Candidate,
            &BTreeSet::from(["operation.1".to_owned()]),
            &planned(),
            &open,
        )
        .unwrap();

        assert!(verify(CoverageMode::Candidate, &BTreeSet::new(), &planned(), &open).is_err());
    }

    #[test]
    fn exposed_rejects_missing_e2e_trace_and_facet_evidence() {
        for collector in ["e2e", "trace", "facet"] {
            let mut mismatch = CoverageCollectors::default();
            register_all(&mut mismatch, ["run.show"]).unwrap();
            mismatch.sets.get_mut(collector).unwrap().clear();
            assert!(
                verify(
                    CoverageMode::Exposed,
                    &BTreeSet::new(),
                    &planned(),
                    &mismatch
                )
                .is_err(),
                "missing {collector} evidence must fail closed"
            );
        }
    }

    fn write_facet(root: &std::path::Path, name: &str, value: serde_json::Value) {
        let directory = root.join("quality/facets/v1");
        fs::create_dir_all(&directory).unwrap();
        let test_directory = root.join("crates/loop-engine-cli/tests");
        let evidence_directory = test_directory.join("e2e");
        fs::create_dir_all(&evidence_directory).unwrap();
        fs::write(
            test_directory.join("e2e.rs"),
            "#[path = \"e2e/proof.rs\"]\nmod proof;\n",
        )
        .unwrap();
        fs::write(
            evidence_directory.join("proof.rs"),
            "#[test]\nfn valid_path() {}\n",
        )
        .unwrap();
        fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    fn valid_facet(operation: &str) -> serde_json::Value {
        let facets = required_facets(operation)
            .unwrap()
            .iter()
            .map(|name| {
                let evidence = if name.starts_with("Trace ") {
                    "trace:e2e:proof::valid_path"
                } else {
                    "e2e:proof::valid_path"
                };
                json!({
                    "name": name,
                    "status": "closed",
                    "evidence": [evidence]
                })
            })
            .collect::<Vec<_>>();
        json!({
            "schema_version": 1,
            "operation_id": operation,
            "facets": facets
        })
    }

    #[test]
    fn facet_manifest_canaries_reject_missing_open_and_stale_artifacts() {
        let missing = tempfile::tempdir().unwrap();
        fs::create_dir_all(missing.path().join("quality/facets/v1")).unwrap();
        assert!(read_facet_manifests(missing.path(), &["run.show".into()]).is_err());

        let open = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.show");
        value["facets"][0]["status"] = json!("open");
        write_facet(open.path(), "run.show", value);
        assert!(read_facet_manifests(open.path(), &["run.show".into()]).is_err());

        let stale = tempfile::tempdir().unwrap();
        write_facet(stale.path(), "run.show", valid_facet("run.show"));
        let mut stale_value = valid_facet("run.show");
        stale_value["operation_id"] = json!("run.graph");
        write_facet(stale.path(), "run.graph", stale_value);
        assert!(read_facet_manifests(stale.path(), &["run.show".into()]).is_err());
    }

    #[test]
    fn facet_manifest_canaries_reject_filename_mismatch_duplicate_names_and_empty_evidence() {
        let mismatch = tempfile::tempdir().unwrap();
        write_facet(mismatch.path(), "run.show", valid_facet("run.list"));
        assert!(read_facet_manifests(mismatch.path(), &["run.show".into()]).is_err());

        let duplicate = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.show");
        value["facets"] = json!([
            {"name": UNIVERSAL_FACET, "status": "closed", "evidence": ["e2e:a"]},
            {"name": UNIVERSAL_FACET, "status": "closed", "evidence": ["e2e:b"]}
        ]);
        write_facet(duplicate.path(), "run.show", value);
        assert!(read_facet_manifests(duplicate.path(), &["run.show".into()]).is_err());

        let empty = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.show");
        value["facets"][0]["evidence"] = json!([]);
        write_facet(empty.path(), "run.show", value);
        assert!(read_facet_manifests(empty.path(), &["run.show".into()]).is_err());

        let missing_required = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.request");
        value["facets"]
            .as_array_mut()
            .unwrap()
            .retain(|facet| facet["name"].as_str() != Some("Gate driven"));
        write_facet(missing_required.path(), "run.request", value);
        assert!(read_facet_manifests(missing_required.path(), &["run.request".into()]).is_err());

        let stale_reference = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.show");
        value["facets"][0]["evidence"] = json!(["e2e:proof::deleted_test"]);
        write_facet(stale_reference.path(), "run.show", value);
        assert!(read_facet_manifests(stale_reference.path(), &["run.show".into()]).is_err());

        let untyped_trace = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.show");
        let trace_facet = value["facets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|facet| facet["name"] == "Trace persistence boundary")
            .unwrap();
        trace_facet["evidence"] = json!(["e2e:proof::valid_path"]);
        write_facet(untyped_trace.path(), "run.show", value);
        assert!(read_facet_manifests(untyped_trace.path(), &["run.show".into()]).is_err());

        let trace_mislabeled_as_behavior = tempfile::tempdir().unwrap();
        let mut value = valid_facet("run.show");
        value["facets"][0]["evidence"] = json!(["trace:e2e:proof::valid_path"]);
        write_facet(trace_mislabeled_as_behavior.path(), "run.show", value);
        assert!(
            read_facet_manifests(trace_mislabeled_as_behavior.path(), &["run.show".into()])
                .is_err()
        );

        for source in [
            "fn valid_path() {}\n",
            "// #[test]\n// fn valid_path() {}\n",
            "#[ignore]\n#[test]\nfn valid_path() {}\n",
            "#[cfg(any())]\n#[test]\nfn valid_path() {}\n",
            "#[cfg(any())]\nmod disabled { #[test] fn valid_path() {} }\n",
        ] {
            let dead_test = tempfile::tempdir().unwrap();
            write_facet(dead_test.path(), "run.show", valid_facet("run.show"));
            fs::write(
                dead_test
                    .path()
                    .join("crates/loop-engine-cli/tests/e2e/proof.rs"),
                source,
            )
            .unwrap();
            assert!(read_facet_manifests(dead_test.path(), &["run.show".into()]).is_err());
        }

        let unregistered = tempfile::tempdir().unwrap();
        write_facet(unregistered.path(), "run.show", valid_facet("run.show"));
        fs::write(
            unregistered
                .path()
                .join("crates/loop-engine-cli/tests/e2e.rs"),
            "",
        )
        .unwrap();
        assert!(read_facet_manifests(unregistered.path(), &["run.show".into()]).is_err());

        let repointed = tempfile::tempdir().unwrap();
        write_facet(repointed.path(), "run.show", valid_facet("run.show"));
        fs::write(
            repointed.path().join("crates/loop-engine-cli/tests/e2e.rs"),
            "#[path = \"e2e/replacement.rs\"]\nmod proof;\n",
        )
        .unwrap();
        assert!(read_facet_manifests(repointed.path(), &["run.show".into()]).is_err());

        let disabled_module = tempfile::tempdir().unwrap();
        write_facet(disabled_module.path(), "run.show", valid_facet("run.show"));
        fs::write(
            disabled_module
                .path()
                .join("crates/loop-engine-cli/tests/e2e.rs"),
            "#[cfg(any())]\n#[path = \"e2e/proof.rs\"]\nmod proof;\n",
        )
        .unwrap();
        assert!(read_facet_manifests(disabled_module.path(), &["run.show".into()]).is_err());

        let disabled_parent = tempfile::tempdir().unwrap();
        write_facet(disabled_parent.path(), "run.show", valid_facet("run.show"));
        fs::write(
            disabled_parent
                .path()
                .join("crates/loop-engine-cli/tests/e2e.rs"),
            "#[cfg(any())]\nmod disabled {\n    #[path = \"e2e/proof.rs\"]\n    mod proof;\n}\n",
        )
        .unwrap();
        assert!(read_facet_manifests(disabled_parent.path(), &["run.show".into()]).is_err());

        let enabled_parent = tempfile::tempdir().unwrap();
        write_facet(enabled_parent.path(), "run.show", valid_facet("run.show"));
        fs::write(
            enabled_parent
                .path()
                .join("crates/loop-engine-cli/tests/e2e.rs"),
            "mod wrapper {\n    #[path = \"e2e/proof.rs\"]\n    mod proof;\n}\n",
        )
        .unwrap();
        assert!(read_facet_manifests(enabled_parent.path(), &["run.show".into()]).is_err());
    }
}
