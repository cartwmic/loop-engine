//! Framework-free workflow model and application boundary.
//!
//! Dependencies point inward: operations may use capabilities and model;
//! capabilities may use model; model depends on neither. Core owns generic
//! workflow facts and deterministic transition decisions only. Provider policy,
//! serialization, subprocesses, SQLite, filesystem layout, and CLI DTOs belong
//! to outer crates.
//!
//! Stored run state is authoritative. Journal facts explain observed activity;
//! they are not replay authority. Gate and guidance providers report bounded
//! observations but cannot choose target state. Passive model inspection never
//! executes provider code.

pub mod capabilities;
pub mod model;
pub mod operations;
