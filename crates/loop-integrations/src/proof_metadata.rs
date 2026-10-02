//! Deterministic repository identity and read-only historical receipt verification.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::{ffi::OsStrExt, fs::PermissionsExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_json(path: &Path) -> io::Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub inherit_environment: Vec<String>,
    pub timeout_ms: u64,
    pub obligations: Vec<String>,
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

fn settings(row: &Row) -> Value {
    let inherited: BTreeMap<_, _> = row
        .inherit_environment
        .iter()
        .map(|key| (key, std::env::var(key).ok()))
        .collect();
    json!({"environment":row.environment,"inherited_environment":inherited,"timeout_ms":row.timeout_ms})
}
fn resolve(row: &Row, cwd: &Path) -> Option<PathBuf> {
    let executable = &row.argv[0];
    let candidate = |p: PathBuf| {
        if p.is_file() && fs::metadata(&p).ok()?.permissions().mode() & 0o111 != 0 {
            fs::canonicalize(p).ok()
        } else {
            None
        }
    };
    if executable.contains('/') {
        return candidate(cwd.join(executable));
    }
    let path = row
        .environment
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())?;
    std::env::split_paths(&path).find_map(|p| candidate(cwd.join(p).join(executable)))
}
/// Read a selected immutable receipt without launching, resuming or changing it.
pub fn verify_selected(index_path: &Path, row: &Row, cwd: &Path) -> io::Result<(PathBuf, Value)> {
    let index = read_json(index_path)?;
    let matches: Vec<_> = index["receipts"]
        .as_array()
        .ok_or_else(|| error("missing capture selection"))?
        .iter()
        .filter(|s| s["id"] == row.id)
        .collect();
    if matches.len() != 1 {
        return Err(error("missing or duplicate selected receipt"));
    }
    let root = index_path
        .parent()
        .ok_or_else(|| error("missing index parent"))?;
    let path = root
        .join(
            matches[0]["receipt"]
                .as_str()
                .ok_or_else(|| error("missing receipt path"))?,
        )
        .canonicalize()?;
    if !path.starts_with(root.canonicalize()?.join("attempts")) {
        return Err(error("receipt escapes capture attempts"));
    }
    let receipt = read_json(&path)?;
    let schema: Value = serde_json::from_str(include_str!(
        "../../../docs/operational-ux-capture.schema.json"
    ))?;
    if !jsonschema::validator_for(&schema["$defs"]["receipt"])
        .map_err(|e| error(e.to_string()))?
        .is_valid(&receipt)
    {
        return Err(error("malformed capture receipt"));
    }
    let identity = repository_proof_identity(cwd)?;
    if receipt["id"] != row.id
        || receipt["argv"] != json!(row.argv)
        || receipt["cwd"] != json!(cwd)
        || receipt["settings"] != settings(row)
        || receipt["obligations"] != json!(row.obligations)
        || receipt["resolved_executable"] != json!(resolve(row, cwd))
        || receipt["repository_before"] != identity
        || receipt["repository_after"] != identity
        || receipt["finished_at"].as_f64() < receipt["started_at"].as_f64()
        || !success(&receipt)
    {
        return Err(error("stale, mismatched or failed selected capture"));
    }
    let attempt = path.parent().unwrap();
    let started = read_json(&attempt.join("started.json"))?;
    for key in [
        "id",
        "argv",
        "resolved_executable",
        "cwd",
        "settings",
        "obligations",
        "repository_before",
        "started_at",
    ] {
        if started.get(key).is_none() || started[key] != receipt[key] {
            return Err(error("started/finished mismatch"));
        }
    }
    for stream in ["stdout", "stderr"] {
        let stream_path = attempt
            .join(
                receipt[stream]
                    .as_str()
                    .ok_or_else(|| error("missing stream"))?,
            )
            .canonicalize()?;
        if !stream_path.starts_with(attempt)
            || receipt[format!("{stream}_sha256")] != hash(&fs::read(stream_path)?)
        {
            return Err(error("capture stream path/digest mismatch"));
        }
    }
    Ok((path, receipt))
}

fn success(r: &Value) -> bool {
    r["exit_code"] == 0
        && r["signal"].is_null()
        && r["timed_out"] == false
        && r["aborted"] != true
        && r["spawn_error"].is_null()
        && r["capture_error"].is_null()
        && r["cleanup"] == "complete"
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
