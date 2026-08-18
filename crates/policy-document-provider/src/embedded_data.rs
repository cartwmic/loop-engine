use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

pub struct EmbeddedFile {
    pub path: &'static str,
    pub bytes: &'static [u8],
}
pub static FILES: &[EmbeddedFile] = &[
    EmbeddedFile {
        path: "crates/policy-document-provider/data/readme.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/readme.json")),
    },
    EmbeddedFile {
        path: "crates/policy-document-provider/data/agents.json",
        bytes: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/agents.json")),
    },
    EmbeddedFile {
        path: "crates/policy-document-provider/data/reviewer-protocol.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/reviewer-protocol.md"
        )),
    },
    EmbeddedFile {
        path: "crates/policy-document-provider/data/semantic-review-worker-preamble.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/semantic-review-worker-preamble.md"
        )),
    },
    EmbeddedFile {
        path: "crates/policy-document-provider/data/semantic-review-worker-output-schema.json",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/semantic-review-worker-output-schema.json"
        )),
    },
    EmbeddedFile {
        path: "crates/policy-document-provider/data/target-guidance.md",
        bytes: include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/target-guidance.md"
        )),
    },
];
pub fn dump(destination: &Path) -> io::Result<()> {
    dump_files(destination, FILES, None)
}

fn dump_files(
    destination: &Path,
    files: &[EmbeddedFile],
    fail_after: Option<usize>,
) -> io::Result<()> {
    let targets: Vec<(&EmbeddedFile, PathBuf)> = files
        .iter()
        .map(|f| (f, destination.join(f.path)))
        .collect();
    for (_, target) in &targets {
        if fs::symlink_metadata(target).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to overwrite existing target {}", target.display()),
            ));
        }
        if let Some(parent) = target.parent() {
            ensure_destination_components_are_safe(destination, parent)?;
            if fs::symlink_metadata(parent).is_ok_and(|metadata| !metadata.is_dir()) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("parent is not directory: {}", parent.display()),
                ));
            }
        }
    }
    let mut created = Vec::new();
    let result = (|| {
        for (_, target) in &targets {
            fs::create_dir_all(target.parent().expect("embedded path parent"))?;
        }
        for (index, (file, target)) in targets.iter().enumerate() {
            if fail_after == Some(index) {
                return Err(io::Error::other("injected dump failure"));
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            created.push(target.clone());
            std::io::Write::write_all(&mut output, file.bytes)?;
        }
        Ok::<(), io::Error>(())
    })();
    if result.is_err() {
        // Remove only files successfully created by this invocation.
        for target in created {
            let _ = fs::remove_file(target);
        }
    }
    result
}

fn ensure_destination_components_are_safe(destination: &Path, parent: &Path) -> io::Result<()> {
    let relative = parent.strip_prefix(destination).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("target parent escapes destination: {}", parent.display()),
        )
    })?;
    let mut current = destination.to_path_buf();
    match fs::symlink_metadata(&current) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "refusing symlink destination component {}",
                    current.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("target parent escapes destination: {}", parent.display()),
                ));
            }
            _ => current.push(component.as_os_str()),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "refusing symlink destination component {}",
                        current.display()
                    ),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn destination() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "policy-data-dump-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn manifest_exactly_matches_focused_data() {
        let paths = FILES.iter().map(|file| file.path).collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                "crates/policy-document-provider/data/readme.json",
                "crates/policy-document-provider/data/agents.json",
                "crates/policy-document-provider/data/reviewer-protocol.md",
                "crates/policy-document-provider/data/semantic-review-worker-preamble.md",
                "crates/policy-document-provider/data/semantic-review-worker-output-schema.json",
                "crates/policy-document-provider/data/target-guidance.md",
            ]
        );
        for file in FILES {
            assert!(!file.bytes.is_empty(), "{}", file.path);
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(file.path);
            assert_eq!(fs::read(source).unwrap(), file.bytes);
        }
    }

    #[test]
    fn reviewer_protocol_scripts_complete_frozen_evidence_shape() {
        let protocol = std::str::from_utf8(
            FILES
                .iter()
                .find(|file| file.path.ends_with("reviewer-protocol.md"))
                .unwrap()
                .bytes,
        )
        .unwrap();
        for field in [
            "gate",
            "policy_id",
            "result",
            "findings",
            "author",
            "target_id",
            "target_sha256",
            "profile_version",
        ] {
            assert!(
                protocol.contains(&format!("\"{field}\"")),
                "missing {field}"
            );
        }
        assert!(protocol.contains("64 lowercase hexadecimal SHA-256"));
        assert!(protocol.contains("human"));
        assert!(protocol.contains("agent"));
        assert!(protocol.contains("script"));
    }

    #[test]
    fn semantic_review_worker_contract_is_complete() {
        let preamble = std::str::from_utf8(
            FILES
                .iter()
                .find(|file| file.path.ends_with("semantic-review-worker-preamble.md"))
                .unwrap()
                .bytes,
        )
        .unwrap();
        for phrase in [
            "read-only",
            "Judge only the assigned axis.",
            "driver context only",
            "axis, author, result, and findings",
            "Do not perform driver duties.",
            "deterministic",
            "show",
            "append",
            "event",
            "progress",
        ] {
            assert!(preamble.contains(phrase), "missing {phrase}");
        }

        let schema_file = FILES
            .iter()
            .find(|file| {
                file.path
                    .ends_with("semantic-review-worker-output-schema.json")
            })
            .unwrap();
        let schema: serde_json::Value = serde_json::from_slice(schema_file.bytes).unwrap();
        assert_eq!(
            schema,
            serde_json::json!({"required": ["axis", "author", "result", "findings"]})
        );
    }

    #[test]
    fn dump_materializes_exact_bytes_and_refuses_overwrite() {
        let root = destination();
        dump(&root).unwrap();
        for file in FILES {
            assert_eq!(fs::read(root.join(file.path)).unwrap(), file.bytes);
        }
        let error = dump(&root).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        for file in FILES {
            assert_eq!(fs::read(root.join(file.path)).unwrap(), file.bytes);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_occupied_and_survives_failed_dump() {
        use std::os::unix::fs::symlink;
        let root = destination();
        let target = root.join("crates/policy-document-provider/data/readme.json");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        symlink("missing-owner-file", &target).unwrap();
        assert_eq!(
            dump(&root).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn intermediate_directory_symlink_is_rejected_without_writing_outside_destination() {
        use std::os::unix::fs::symlink;
        let root = destination();
        let outside = destination();
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("crates")).unwrap();

        assert_eq!(
            dump(&root).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert!(!outside.join("policy-document-provider").exists());
        assert!(fs::symlink_metadata(root.join("crates"))
            .unwrap()
            .file_type()
            .is_symlink());

        fs::remove_file(root.join("crates")).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn rollback_removes_only_files_created_by_this_invocation() {
        let root = destination();
        let owned = root.join("caller-owned.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&owned, b"keep").unwrap();
        assert!(dump_files(&root, FILES, Some(1)).is_err());
        assert_eq!(fs::read(&owned).unwrap(), b"keep");
        assert!(!root.join(FILES[0].path).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dump_preflight_failure_leaves_destination_untouched() {
        let root = destination();
        fs::create_dir_all(&root).unwrap();
        let blocked = root.join("crates");
        fs::write(&blocked, b"not-a-directory").unwrap();
        assert!(dump(&root).is_err());
        assert!(blocked.is_file());
        assert_eq!(fs::read(&blocked).unwrap(), b"not-a-directory");
        fs::remove_dir_all(root).unwrap();
    }
}
