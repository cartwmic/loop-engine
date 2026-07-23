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
fn runtime_catalogs_match_provider_foundation_exposure() {
    let exposed = ["provider.add", "provider.list"];
    assert_eq!(DRIVER_OPERATION_IDS, exposed);
    assert_eq!(REACHABLE_ROUTE_OPERATION_IDS, exposed);
    assert_eq!(E2E_OPERATION_IDS, exposed);
    assert_eq!(TRACE_OPERATION_IDS, exposed);
    assert_eq!(FACET_OPERATION_IDS, exposed);
    assert_eq!(driver_operations().len(), 2);
    assert_eq!(reachable_route_operations().len(), 2);
    assert_eq!(e2e_operations().len(), 2);
    assert_eq!(trace_operations().len(), 2);
    assert_eq!(facet_operations().len(), 2);
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
