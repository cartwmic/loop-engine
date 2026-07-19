//! Bypass fixture integrations crate.

pub fn spawn_provider() {
    let _child = std::process::Command::new("true").spawn();
}

pub mod store;
