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

pub(crate) fn from_artifacts(root: &Path, cwd: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let intent = match std::fs::read(root.join("intent.json")) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|e| e.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(error) => return Err(error.to_string()),
    };
    roots(intent.get("source_scopes"), cwd)
}

pub(crate) fn resolve(roots: &BTreeMap<String, PathBuf>, scope: &str) -> Result<PathBuf, String> {
    roots
        .get(scope)
        .cloned()
        .ok_or_else(|| format!("unknown source scope `{scope}`"))
}
