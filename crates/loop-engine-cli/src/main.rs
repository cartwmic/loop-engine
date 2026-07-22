//! Composition root entry (T120).

// WP1 intentionally builds delivery substrate before exposing application routes.
// Keep dead-code allowance scoped to private pre-exposure modules; remove it as WP2+ wires routes.
#[allow(
    dead_code,
    reason = "private WP1 command surface awaits route exposure"
)]
mod args;
#[allow(
    dead_code,
    reason = "private WP1 operation adapters await route exposure"
)]
mod commands;
#[allow(
    dead_code,
    reason = "private WP1 composition root awaits route exposure"
)]
mod composition;
#[allow(dead_code, reason = "private WP1 diagnostics await route exposure")]
mod diagnostics;
#[allow(dead_code, reason = "private WP1 dispatcher awaits route exposure")]
mod dispatch;
#[allow(dead_code, reason = "WP1 operation lists are intentionally empty")]
mod driver_catalog;
#[allow(dead_code, reason = "private WP1 exit policy awaits route exposure")]
mod exit;
#[allow(dead_code, reason = "private WP1 renderers await route exposure")]
mod render;
mod startup;

fn main() {
    std::process::exit(startup::run());
}
