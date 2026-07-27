//! Production driver registry and facet closure invariants.

#[path = "../src/driver_catalog.rs"]
mod driver_catalog;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use driver_catalog::DRIVER_OPERATIONS;

fn facet_manifest_operations() -> BTreeSet<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../quality/facets/v1");
    fs::read_dir(root)
        .expect("read facet manifest directory")
        .filter_map(|entry| {
            let path = entry.expect("read facet manifest entry").path();
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || path.file_name().and_then(|value| value.to_str()) == Some("schema.json")
            {
                return None;
            }

            let file_operation = path
                .file_stem()
                .and_then(|value| value.to_str())
                .expect("facet manifest filename must be UTF-8")
                .to_owned();
            let document: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).expect("read facet manifest"))
                    .expect("facet manifest must be JSON");
            assert_eq!(
                document["operation_id"].as_str(),
                Some(file_operation.as_str()),
                "facet operation_id must match filename: {}",
                path.display()
            );
            Some(file_operation)
        })
        .collect()
}

#[test]
fn production_driver_registry_exactly_matches_operation_catalog() {
    let driver_ids = DRIVER_OPERATIONS
        .iter()
        .map(|operation| operation.id)
        .collect::<Vec<_>>();
    assert_eq!(
        driver_ids,
        loop_engine_core::operations::catalog::PLANNED_OPERATION_IDS
    );
    assert_eq!(
        driver_ids.iter().copied().collect::<BTreeSet<_>>().len(),
        driver_ids.len(),
        "production driver IDs must be unique"
    );
    assert!(
        DRIVER_OPERATIONS
            .iter()
            .all(|operation| !operation.argv.is_empty()),
        "every production driver entry must provide its rendered argv template"
    );
}

#[test]
fn facet_manifest_inventory_exactly_matches_operation_catalog() {
    let catalog = loop_engine_core::operations::catalog::PLANNED_OPERATION_IDS
        .iter()
        .map(|operation| (*operation).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(facet_manifest_operations(), catalog);
}
