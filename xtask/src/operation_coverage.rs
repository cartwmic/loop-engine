use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

const REQUIRED_COLLECTORS: &[&str] = &["core", "driver", "route", "e2e", "trace", "facet"];
const CATALOG_COLLECTORS: &[&str] = &["driver", "route"];
const EVIDENCE_COLLECTORS: &[&str] = &["e2e", "trace"];

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
    collectors.register(
        "facet",
        collect_array(
            &driver_source,
            "pub const FACET_OPERATION_IDS: &[&str] = &[",
            "facet operation collector",
        )?,
    )?;
    Ok(collectors)
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

    use super::{CoverageCollectors, CoverageMode, REQUIRED_COLLECTORS, verify};

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
    fn exposed_requires_evidence_and_facet_equality() {
        let mut mismatch = CoverageCollectors::default();
        register_all(&mut mismatch, ["run.show"]).unwrap();
        mismatch.sets.get_mut("e2e").unwrap().clear();
        assert!(
            verify(
                CoverageMode::Exposed,
                &BTreeSet::new(),
                &planned(),
                &mismatch
            )
            .is_err()
        );
    }
}
