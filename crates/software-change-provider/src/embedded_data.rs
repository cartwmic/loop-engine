//! Embedded software-change provider data and safe materialization.
//!
//! `FILES` is the single authoritative list of shipped data. Keep every path
//! relative to repository root so dumped data preserves references emitted by
//! provider guidance and review prompts.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// One file embedded in the provider binary.
#[derive(Debug)]
pub struct EmbeddedFile {
    /// Repository-relative path, including the `crates/software-change-provider/` prefix.
    pub path: &'static str,
    /// Exact bytes committed at the path when this binary was built.
    pub bytes: &'static [u8],
}

/// Complete shipped-data manifest. This is intentionally explicit: adding a
/// data file requires adding its include here, and the drift-guard test catches
/// omissions or stale bytes.
pub static FILES: &[EmbeddedFile] = &[
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/returned-v16.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/returned-v16.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-133.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-133.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-133.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-133.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-134.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-134.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-134.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-134.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/design-narrative-context-narrative-faithful.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/design-narrative-context-narrative-faithful.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/design-narrative-context-narrative-intent-aligned.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/design-narrative-context-narrative-intent-aligned.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/design-narrative-context-narrative-understandable.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/design-narrative-context-narrative-understandable.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-101.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-101.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-101.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-101.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-102.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-102.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-102.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-102.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-103.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-103.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-103.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-103.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-111.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-111.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-111.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-111.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-112.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-112.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-112.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-112.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-113.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-113.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-113.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-113.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-121.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-121.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-121.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-121.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-122.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-122.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-122.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-122.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-123.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-123.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-123.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-123.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-131.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-131.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-131.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-131.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-132.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-132.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/narrative-132.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/narrative-132.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/plan-narrative-context-narrative-faithful.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/plan-narrative-context-narrative-faithful.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/plan-narrative-context-narrative-intent-aligned.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/plan-narrative-context-narrative-intent-aligned.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/plan-narrative-context-narrative-understandable.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/plan-narrative-context-narrative-understandable.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/validation-narrative-context-narrative-faithful.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/validation-narrative-context-narrative-faithful.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/validation-narrative-context-narrative-intent-aligned.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/validation-narrative-context-narrative-intent-aligned.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/validation-narrative-context-narrative-understandable.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/validation-narrative-context-narrative-understandable.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/acceptance-sufficient-good.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/acceptance-sufficient-good.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/intent-sufficient-good.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/intent-sufficient-good.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/intent-sufficient-defective.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/intent-sufficient-defective.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/fixtures/acceptance-sufficient-defective.json", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/acceptance-sufficient-defective.json")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/companions/fictional-repo/implementation-evidence/acceptance-sufficient-coverage.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/companions/fictional-repo/implementation-evidence/acceptance-sufficient-coverage.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/companions/fictional-repo/implementation-evidence/intent-sufficient-comparison.md", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/companions/fictional-repo/implementation-evidence/intent-sufficient-comparison.md")) },
    EmbeddedFile { path: "crates/software-change-provider/data/calibration/companions/fictional-repo/implementation-evidence/baseline-2026-08-12.txt", bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/companions/fictional-repo/implementation-evidence/baseline-2026-08-12.txt")) },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/validation-evidence-2026-08-12.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/validation-evidence-2026-08-12.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/validation-evidence-2026-08-13.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/calibration/fixtures/validation-evidence-2026-08-13.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/validation-report-schema.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/validation-report-schema.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/narrative-schema.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/narrative-schema.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/narrative.md",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/templates/narrative.md")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/reconciliation-schema.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/reconciliation-schema.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/recovery-contract-v2.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/recovery-contract-v2.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/PROCEDURE.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/PROCEDURE.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/reviewer-instruction.txt",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/reviewer-instruction.txt"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/README.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/README.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/implementation-evidence/repo-state-2026-08-12.txt",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/implementation-evidence/repo-state-2026-08-12.txt"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/implementation-evidence/repo-state-2026-08-13.txt",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/implementation-evidence/repo-state-2026-08-13.txt"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/implementation-evidence/requirement-to-proof.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/implementation-evidence/requirement-to-proof.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/docs/PRD.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/docs/PRD.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/loop-engine-software-change-provider-prd.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/loop-engine-software-change-provider-prd.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/loop-engine-software-change-provider-task-packets.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/loop-engine-software-change-provider-task-packets.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/loop-engine-software-change-provider-technical-design.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/loop-engine-software-change-provider-technical-design.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/provider/README.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/provider/README.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/scripts/assert-doc-authority.py",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/scripts/assert-doc-authority.py"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/scripts/assert-requirement-proof.py",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/scripts/assert-requirement-proof.py"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/scripts/production-journey.py",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/scripts/production-journey.py"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/docs/review-contract.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/docs/review-contract.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/companions/fictional-repo/docs/requirement-coverage.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/companions/fictional-repo/docs/requirement-coverage.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/design-defective.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/design-defective.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/design-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/design-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/design-layered.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/design-layered.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/design-overbuilt.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/design-overbuilt.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/example-evidence.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/example-evidence.json"
        )),
    },
    EmbeddedFile {
        path:
            "crates/software-change-provider/data/calibration/fixtures/implementation-report-defective.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/implementation-report-defective.json"
        )),
    },
    EmbeddedFile {
        path:
            "crates/software-change-provider/data/calibration/fixtures/implementation-report-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/implementation-report-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-defective.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-defective.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-granularity-bundled.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-granularity-bundled.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-granularity-fragmented.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-granularity-fragmented.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-granularity-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-granularity-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-granularity-impractical.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-granularity-impractical.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-owner-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-owner-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/intent-owner-unstated-background.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/intent-owner-unstated-background.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/requirement-coverage-implementation-defect.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/requirement-coverage-implementation-defect.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/requirement-coverage-related-insufficient.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/requirement-coverage-related-insufficient.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/requirement-coverage-sufficient.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/requirement-coverage-sufficient.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/plan-collision.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/plan-collision.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/plan-defective.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/plan-defective.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/plan-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/plan-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/plan-missed-parallelism.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/plan-missed-parallelism.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/plan-oversplit.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/plan-oversplit.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/fixtures/plan-serial-honest.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/plan-serial-honest.json"
        )),
    },
    EmbeddedFile {
        path:
            "crates/software-change-provider/data/calibration/fixtures/validation-report-defective.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/validation-report-defective.json"
        )),
    },
    EmbeddedFile {
        path:
            "crates/software-change-provider/data/calibration/fixtures/validation-report-good.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/fixtures/validation-report-good.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/manifest.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/manifest.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v12.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v12.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v14.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v14.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v14-r2.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v14-r2.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v14-r3.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v14-r3.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v14-r4.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v14-r4.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v15.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v15.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v15-r1.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v15-r1.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v15-r2.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v15-r2.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/calibration/returned-v15-r3.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/calibration/returned-v15-r3.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/configs/high-rigor.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/configs/high-rigor.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/configs/minimal.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/configs/minimal.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/configs/standard.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/configs/standard.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/review-worker-output-schema.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/review-worker-output-schema.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/review-worker-output-schema-v2.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/review-worker-output-schema-v2.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/review-worker-output-schema-v3.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/review-worker-output-schema-v3.json")),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/review-worker-preamble.txt",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/review-worker-preamble.txt"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/reviewer-protocol.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/reviewer-protocol.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/finding-ledger.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/finding-ledger.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/advisory-finding-proposal.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/advisory-finding-proposal.json"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/design.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/design.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/implementation-report.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/implementation-report.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/intent.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/intent.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/task-packet.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/task-packet.md"
        )),
    },
    EmbeddedFile {
        path: "crates/software-change-provider/data/templates/validation-report.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/validation-report.md"
        )),
    },
];

/// Materialize all embedded files beneath `destination`.
///
/// Existing target files are rejected during a complete preflight before any
/// target file or directory is written. Existing parent directories are safe;
/// all target paths retain their repository-relative layout.
pub fn dump(destination: &Path) -> io::Result<()> {
    let targets: Vec<(&EmbeddedFile, PathBuf)> = FILES
        .iter()
        .map(|file| (file, destination.join(file.path)))
        .collect();

    for (_, target) in &targets {
        if path_exists(target)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to overwrite existing target {}", target.display()),
            ));
        }
        ensure_parent_paths_are_directories(target.parent().expect("embedded target has parent"))?;
    }

    for (_, target) in &targets {
        fs::create_dir_all(target.parent().expect("embedded target has parent"))?;
    }

    for (file, target) in targets {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("could not create {}: {error}", target.display()),
                )
            })?;
        output.write_all(file.bytes).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not write {}: {error}", target.display()),
            )
        })?;
    }

    Ok(())
}

fn path_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn ensure_parent_paths_are_directories(path: &Path) -> io::Result<()> {
    for ancestor in path.ancestors() {
        match fs::metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("dump parent is not a directory: {}", ancestor.display()),
                ))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_parallelism_fixtures_match_shipped_artifact_schemas() {
        for (fixture, subject, revision_link) in [
            ("design-layered", "design.json", "intent_revision"),
            ("plan-missed-parallelism", "plan.json", "design_revision"),
            ("plan-collision", "plan.json", "design_revision"),
            ("plan-oversplit", "plan.json", "design_revision"),
            ("plan-serial-honest", "plan.json", "design_revision"),
        ] {
            let path =
                format!("crates/software-change-provider/data/calibration/fixtures/{fixture}.json");
            let file = FILES
                .iter()
                .find(|file| file.path == path)
                .expect("fixture is in FILES");
            let instance: serde_json::Value = serde_json::from_slice(file.bytes).unwrap();
            assert_eq!(instance["revision"], "r15", "{fixture}");
            assert_eq!(instance[revision_link], "r15", "{fixture}");
            for profile in ["minimal", "standard", "high-rigor"] {
                let path = format!("crates/software-change-provider/data/configs/{profile}.json");
                let file = FILES.iter().find(|file| file.path == path).unwrap();
                let config: serde_json::Value = serde_json::from_slice(file.bytes).unwrap();
                let schema =
                    crate::schema::validate_schema(&config["artifact_schemas"][subject]).unwrap();
                let report = schema.evaluate(&instance);
                assert!(report.is_valid(), "{profile}/{fixture}: {report:?}");
            }
        }
    }

    #[test]
    fn embedded_narrative_contract_matches_source_bytes() {
        for (path, source) in [
            (
                "crates/software-change-provider/data/narrative-schema.json",
                include_bytes!("../data/narrative-schema.json").as_slice(),
            ),
            (
                "crates/software-change-provider/data/templates/narrative.md",
                include_bytes!("../data/templates/narrative.md").as_slice(),
            ),
        ] {
            let matches = FILES
                .iter()
                .filter(|file| file.path == path)
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "{path}");
            assert_eq!(matches[0].bytes, source, "{path}");
        }
    }

    #[test]
    fn embedded_profile_inventory_ships_contract_four_explicit_groups() {
        for name in ["minimal", "standard", "high-rigor"] {
            let path = format!("crates/software-change-provider/data/configs/{name}.json");
            let file = FILES
                .iter()
                .find(|file| file.path == path)
                .expect("profile is in FILES");
            let profile: serde_json::Value = serde_json::from_slice(file.bytes).unwrap();
            assert_eq!(profile["contract_version"], 4);
            assert_eq!(profile["config_version"], format!("{name}-16"));
            assert_eq!(profile["extra"]["narratives"], serde_json::json!([]));
            assert_eq!(profile["extra"]["checkpoints"], serde_json::json!([]));
            for entries in profile["review_policies"].as_object().unwrap().values() {
                for entry in entries.as_array().unwrap() {
                    assert!(entry["group"].is_string());
                }
            }
        }
    }
}
