//! Embedded research provider data and safe materialization.
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
    /// Repository-relative path, including the `crates/research-provider/` prefix.
    pub path: &'static str,
    /// Exact bytes committed at the path when this binary was built.
    pub bytes: &'static [u8],
}

/// Complete shipped-data manifest. This is intentionally explicit: adding a
/// data file requires adding its include here, and the drift-guard test catches
/// omissions or stale bytes.
pub static FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        path: "crates/research-provider/AGENTS.md",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md")),
    },
    EmbeddedFile {
        path: "crates/research-provider/README.md",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/configs/standard.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/configs/standard.json"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/reviewer-protocol.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/reviewer-protocol.md"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/review-worker-output-schema.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/review-worker-output-schema.json"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/review-worker-preamble.txt",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/review-worker-preamble.txt"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/templates/brief.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/brief.md"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/templates/report.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/report.md"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/templates/sources.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/sources.md"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/data/templates/verification.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/templates/verification.md"
        )),
    },
    EmbeddedFile {
        path: "crates/research-provider/skills/using-research-provider/SKILL.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/skills/using-research-provider/SKILL.md"
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
