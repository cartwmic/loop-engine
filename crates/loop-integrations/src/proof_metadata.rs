//! Deterministic repository identity and read-only historical receipt verification.
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
use std::{collections::BTreeSet, fs, io, path::Path, process::Command};
fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
pub(crate) fn git(repo: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let result = Command::new("git").args(args).current_dir(repo).output()?;
    if !result.status.success() {
        return Err(error(String::from_utf8_lossy(&result.stderr)));
    }
    Ok(result.stdout)
}
/// Byte-for-byte counterpart of scripts/test_contract.py; public fixture checks parity.
pub fn repository_proof_identity(repo: &Path) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut add = |bytes: &[u8]| {
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    };
    add(&git(repo, &["rev-parse", "HEAD"])?);
    add(&git(repo, &["ls-files", "--stage", "-z"])?);
    add(&git(
        repo,
        &["status", "--porcelain=v1", "--untracked-files=all", "-z"],
    )?);
    let names = git(
        repo,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    for name in names
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect::<BTreeSet<_>>()
    {
        add(name);
        let path = repo.join(std::ffi::OsStr::from_bytes(name));
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                add(b"symlink");
                add(fs::read_link(path)?.as_os_str().as_bytes());
            }
            Ok(meta) if meta.is_file() => {
                add((meta.permissions().mode() & 0o777).to_string().as_bytes());
                add(&fs::read(path)?);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => add(b"deleted"),
            Err(e) => return Err(e),
            _ => {
                return Err(error(format!(
                    "unsupported repository proof entry: {}",
                    path.display()
                )))
            }
        }
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identical_tree_has_identical_identity() {
        let root = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(root.path())
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&["init", "-q"]);
        fs::write(root.path().join("source"), "original").unwrap();
        git(&["add", "source"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ]);
        let before = repository_proof_identity(root.path()).unwrap();
        assert_eq!(before, repository_proof_identity(root.path()).unwrap());
        fs::write(root.path().join("source"), "changed").unwrap();
        assert_ne!(before, repository_proof_identity(root.path()).unwrap());
    }
}
