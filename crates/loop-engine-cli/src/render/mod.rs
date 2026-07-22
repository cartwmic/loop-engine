//! Structured CLI outcome envelope rendering (T125/T126).
//!
//! Maps dispatched [`PublicOutcome`] values into the frozen schema v1 envelope
//! defined in `docs/cli-contract.md`. Human and structured renderers share one
//! envelope construction path. Pre-dispatch failures, exit-code selection, and
//! stdout/stderr wiring belong to [`crate::exit`].

pub mod dto;
pub mod human;
pub mod json;
