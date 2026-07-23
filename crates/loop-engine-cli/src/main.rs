//! Composition root entry (T120).

// Delivery substrate includes private operations staged for later exposure.
// Keep dead-code allowance scoped to modules containing that reviewed private surface.
#[allow(
    dead_code,
    reason = "planned command grammar includes routes staged for later exposure"
)]
mod args;
#[allow(
    dead_code,
    reason = "private operation adapters await their exposure checkpoints"
)]
mod commands;
#[allow(
    dead_code,
    reason = "composition graph includes adapters for later exposure checkpoints"
)]
mod composition;
#[allow(
    dead_code,
    reason = "diagnostic variants support later exposure checkpoints"
)]
mod diagnostics;
#[allow(
    dead_code,
    reason = "dispatch helpers support later exposure checkpoints"
)]
mod dispatch;
#[allow(
    dead_code,
    reason = "coverage catalogs are consumed by deterministic closure and test targets"
)]
mod driver_catalog;
mod execution;
#[allow(dead_code, reason = "exit helpers support later exposure checkpoints")]
mod exit;
#[allow(
    dead_code,
    reason = "render helpers support later exposure checkpoints"
)]
mod render;
mod startup;

fn main() {
    std::process::exit(startup::run());
}
