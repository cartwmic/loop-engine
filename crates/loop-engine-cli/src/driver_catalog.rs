//! Production driver and runtime coverage catalogs (T133+).
//!
//! Driver metadata (`--help`, `--version`, `--list-operations`) is not an
//! application operation. These reviewed arrays enumerate only runtime-exposed
//! driver support, reachable production routes, and mechanical coverage evidence.

use loop_engine_core::operations::catalog::OperationId;

/// Operation IDs the production driver can dispatch. Updated only by exposure tasks.
pub const DRIVER_OPERATION_IDS: &[&str] = &["provider.add", "provider.list"];

/// Operation IDs registered as reachable production routes. Updated only by exposure tasks.
pub const REACHABLE_ROUTE_OPERATION_IDS: &[&str] = &["provider.add", "provider.list"];

/// Operation IDs observed in passing required E2E scenarios. Updated by T145+.
pub const E2E_OPERATION_IDS: &[&str] = &["provider.add", "provider.list"];

/// Operation IDs observed in correlated passing trace files. Updated by T145+.
pub const TRACE_OPERATION_IDS: &[&str] = &["provider.add", "provider.list"];

/// Operation IDs with closed facet manifests. Updated by exposure tasks.
pub const FACET_OPERATION_IDS: &[&str] = &["provider.add", "provider.list"];

fn parse_catalog_ids(values: &[&str]) -> Vec<OperationId> {
    values
        .iter()
        .map(|value| {
            OperationId::parse(value)
                .unwrap_or_else(|_| panic!("catalog ID `{value}` must belong to frozen catalog"))
        })
        .collect()
}

/// Driver-supported operations in stable catalog order.
pub fn driver_operations() -> Vec<OperationId> {
    parse_catalog_ids(DRIVER_OPERATION_IDS)
}

/// Production reachable route operations in stable catalog order.
pub fn reachable_route_operations() -> Vec<OperationId> {
    parse_catalog_ids(REACHABLE_ROUTE_OPERATION_IDS)
}

/// Passing required E2E operations in stable catalog order.
pub fn e2e_operations() -> Vec<OperationId> {
    parse_catalog_ids(E2E_OPERATION_IDS)
}

/// Correlated trace operations in stable catalog order.
pub fn trace_operations() -> Vec<OperationId> {
    parse_catalog_ids(TRACE_OPERATION_IDS)
}

/// Closed facet manifest operations in stable catalog order.
pub fn facet_operations() -> Vec<OperationId> {
    parse_catalog_ids(FACET_OPERATION_IDS)
}
