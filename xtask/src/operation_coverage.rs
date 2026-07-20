use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Result, bail};

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

#[derive(Debug, Default)]
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
    let Some(core) = collectors.sets.get("core") else {
        bail!("operation coverage has no registered core collector");
    };

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

    for (name, values) in &collectors.sets {
        let differences = core
            .symmetric_difference(values)
            .cloned()
            .collect::<BTreeSet<_>>();
        let accepted = if mode == CoverageMode::Candidate {
            differences.is_subset(allow_open)
        } else {
            differences.is_empty()
        };
        if !accepted {
            bail!("operation collector mismatch: `core`={core:?}, `{name}`={values:?}");
        }
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
    let mut root = std::env::current_dir()?;
    while !root
        .join("crates/loop-engine-core/src/operations/catalog.rs")
        .is_file()
    {
        if !root.pop() {
            bail!("could not locate repository root for operation coverage");
        }
    }
    run_at(&root, mode, allow_open)
}

pub fn run_at(root: &Path, mode: CoverageMode, allow_open: &str) -> Result<()> {
    let allow_open = allow_open
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let source =
        fs::read_to_string(root.join("crates/loop-engine-core/src/operations/catalog.rs"))?;
    let planned = collect_array(
        &source,
        "pub const PLANNED_OPERATION_IDS: &[&str] = &[",
        "planned operation catalog",
    )?
    .into_iter()
    .collect::<BTreeSet<_>>();
    let mut collectors = CoverageCollectors::default();
    collectors.register(
        "core",
        collect_array(
            &source,
            "pub const EXPOSED_OPERATION_IDS: &[&str] = &[",
            "core operation collector",
        )?,
    )?;
    verify(mode, &allow_open, &planned, &collectors)
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

    use super::{CoverageCollectors, CoverageMode, verify};

    fn planned() -> BTreeSet<String> {
        std::iter::once("run.show".to_owned())
            .chain((1..21).map(|index| format!("operation.{index}")))
            .collect()
    }

    #[test]
    fn baseline_accepts_empty_and_rejects_nonempty() {
        let mut empty = CoverageCollectors::default();
        empty.register("core", Vec::<String>::new()).unwrap();
        verify(CoverageMode::Baseline, &BTreeSet::new(), &planned(), &empty).unwrap();

        let mut nonempty = CoverageCollectors::default();
        nonempty.register("core", ["run.show"]).unwrap();
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
            verify(
                CoverageMode::Baseline,
                &BTreeSet::new(),
                &planned(),
                &duplicate
            )
            .is_err()
        );

        let mut empty = CoverageCollectors::default();
        empty.register("core", Vec::<String>::new()).unwrap();
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
}
