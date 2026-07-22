//! Driver catalog invariants (T133).

#[path = "../src/driver_catalog.rs"]
mod driver_catalog;

use std::collections::BTreeSet;

use driver_catalog::{
    DRIVER_OPERATION_IDS, E2E_OPERATION_IDS, FACET_OPERATION_IDS, REACHABLE_ROUTE_OPERATION_IDS,
    TRACE_OPERATION_IDS, driver_operations, e2e_operations, facet_operations,
    reachable_route_operations, trace_operations,
};
use loop_engine_core::operations::catalog::OperationId;

fn unique<'a>(values: &[&'a str]) -> BTreeSet<&'a str> {
    values.iter().copied().collect()
}

#[test]
fn runtime_catalogs_start_empty() {
    assert!(DRIVER_OPERATION_IDS.is_empty());
    assert!(REACHABLE_ROUTE_OPERATION_IDS.is_empty());
    assert!(E2E_OPERATION_IDS.is_empty());
    assert!(TRACE_OPERATION_IDS.is_empty());
    assert!(FACET_OPERATION_IDS.is_empty());
    assert!(driver_operations().is_empty());
    assert!(reachable_route_operations().is_empty());
    assert!(e2e_operations().is_empty());
    assert!(trace_operations().is_empty());
    assert!(facet_operations().is_empty());
}

#[test]
fn catalog_ids_are_unique_and_planned() {
    for values in [
        DRIVER_OPERATION_IDS,
        REACHABLE_ROUTE_OPERATION_IDS,
        E2E_OPERATION_IDS,
        TRACE_OPERATION_IDS,
        FACET_OPERATION_IDS,
    ] {
        assert_eq!(values.len(), unique(values).len());
        for value in values {
            assert!(OperationId::parse(value).is_ok());
        }
    }
}
