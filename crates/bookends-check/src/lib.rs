//! Deterministic bookends checker: living git PRD to eligible proof citations.
//!
//! The public interface is `check_repo`, the parser-only candidate helpers,
//! and `CheckReport` / `CheckStatus`. Callers must not reimplement parse,
//! eligibility, continuity, or bypass.

mod check;
mod config;
mod continuity;
mod eligibility;
mod git;
mod prd;

use std::io;
use std::path::Path;

pub use check::check_repo;
pub use prd::{candidate_ids, validate_candidate};

/// Outcome of one checker invocation. Bypass is never a green check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Green,
    Red,
    Bypass { class: String, reason: String },
}

/// Graph evaluation result. `findings` is empty on Green and Bypass and
/// non-empty on Red.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub status: CheckStatus,
    pub live_ids: Vec<String>,
    pub findings: Vec<String>,
}

impl CheckReport {
    pub(crate) fn green(live_ids: Vec<String>) -> Self {
        Self {
            status: CheckStatus::Green,
            live_ids,
            findings: Vec::new(),
        }
    }

    pub(crate) fn red(live_ids: Vec<String>, findings: Vec<String>) -> Self {
        debug_assert!(!findings.is_empty());
        Self {
            status: CheckStatus::Red,
            live_ids,
            findings,
        }
    }

    pub(crate) fn apply_bypass(self, bypass: Option<(&str, &str)>) -> Self {
        match (&self.status, bypass) {
            (CheckStatus::Red, Some((class, reason))) => Self {
                status: CheckStatus::Bypass {
                    class: class.to_string(),
                    reason: reason.to_string(),
                },
                live_ids: self.live_ids,
                findings: Vec::new(),
            },
            _ => self,
        }
    }
}

pub(crate) fn io_err_reading_root(repo_root: &Path, err: io::Error) -> io::Error {
    io::Error::new(
        err.kind(),
        format!("cannot read repo root {}: {err}", repo_root.display()),
    )
}
