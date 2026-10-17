//! Named source roots. Scope-qualified references never fall back to another root.
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(crate) fn roots(
    declaration: Option<&Value>,
    cwd: &Path,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut roots = BTreeMap::new();
    let Some(declaration) = declaration else {
        roots.insert(
            "primary".into(),
            cwd.canonicalize().map_err(|e| e.to_string())?,
        );
        return Ok(roots);
    };
    let scopes = declaration
        .as_object()
        .filter(|s| !s.is_empty())
        .ok_or("source_scopes must be a nonempty object")?;
    for (name, value) in scopes {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err("invalid source scope name".into());
        }
        let object = value
            .as_object()
            .filter(|o| o.len() == 1)
            .ok_or("source scope must contain only root")?;
        let root = Path::new(
            object
                .get("root")
                .and_then(Value::as_str)
                .ok_or("source scope needs root")?,
        );
        if !root.is_absolute() || !root.is_dir() {
            return Err(format!(
                "scope `{name}` root must be an existing absolute directory"
            ));
        }
        roots.insert(
            name.clone(),
            root.canonicalize().map_err(|e| e.to_string())?,
        );
    }
    Ok(roots)
}

fn read_intent(root: &Path) -> Result<Value, String> {
    match std::fs::read(root.join("intent.json")) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn from_artifacts(root: &Path, cwd: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    roots(read_intent(root)?.get("source_scopes"), cwd)
}

/// Refusal wording when a repository-relative locator has no root to resolve against.
pub(crate) const NO_LOCATOR_ROOT: &str = "no source root for repository-relative locators: declare source_scopes in intent.json or keep the artifact directory inside a git work tree";

/// Roots for evidence locators. Never reads the process working directory, so
/// every invocation directory gives the same answer. A declared `source_scopes`
/// wins; otherwise `primary` is the git top level of the artifact directory,
/// and is absent when there is none (callers then refuse with [`NO_LOCATOR_ROOT`]).
pub(crate) fn locator_roots(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let intent = read_intent(root)?;
    if let Some(declaration) = intent.get("source_scopes") {
        return roots(Some(declaration), root);
    }
    let mut roots = BTreeMap::new();
    if let Some(top) = git_top_level(root) {
        roots.insert("primary".into(), top);
    }
    Ok(roots)
}

fn git_top_level(dir: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let text = String::from_utf8(output.stdout).ok()?;
    PathBuf::from(text.trim()).canonicalize().ok()
}

pub(crate) fn resolve(roots: &BTreeMap<String, PathBuf>, scope: &str) -> Result<PathBuf, String> {
    roots
        .get(scope)
        .cloned()
        .ok_or_else(|| format!("unknown source scope `{scope}`"))
}
