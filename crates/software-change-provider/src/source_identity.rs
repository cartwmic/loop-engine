//! Read-only identity shared by external proof producers and verification.
use serde_json::{json, Value};
use std::{path::Path, process::Command};

pub(crate) fn read(cwd: &Path) -> Result<Value, String> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("working directory must be an existing absolute directory".into());
    }
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .output()
        .map_err(|e| e.to_string())?;
    if !head.status.success() {
        return Err(String::from_utf8_lossy(&head.stderr).into_owned());
    }
    let head = String::from_utf8(head.stdout).map_err(|e| e.to_string())?;
    let state = loop_integrations::proof_metadata::repository_proof_identity(cwd)
        .map_err(|e| e.to_string())?;
    Ok(json!({"head":head.trim(),"state_sha256":state}))
}

pub(crate) fn run(args: &[String]) -> Result<Value, String> {
    let cwd = match args {
        [flag, cwd] if flag == "--working-directory" => cwd.as_str(),
        [flag] if flag.starts_with("--working-directory=") => &flag[20..],
        _ => return Err("usage: software-change source-identity --working-directory ABS".into()),
    };
    read(Path::new(cwd))
}
