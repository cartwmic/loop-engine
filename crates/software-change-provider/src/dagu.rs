//! Resolve operator-provided `dagu` from PATH and define the isolated-home locator.
//!
//! Dagu is a GPLv3 binary invoked as a subprocess. This crate does not embed,
//! vendor, or ship it. The resolver fails closed before any worker spawn.

use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Minimum accepted `dagu version` triple.
pub const MINIMUM_DAGU_VERSION: &str = "2.14.0";
const MINIMUM: (u64, u64, u64) = (2, 14, 0);
const DAGU_BIN: &str = "dagu";
const LOCATOR_FILE: &str = "dagu-locator.json";
const HOME_DIR: &str = "dagu-home";
const NAME_PREFIX: &str = "plan-graph";
/// Dagu 2.14.0 rejects graph names of 40 characters or more.
const DAGU_NAME_MAX: usize = 39;

/// Isolated-home locator written at `capture_dir/dagu-locator.json`.
///
/// Exactly these three keys: `dagu_home` (absolute), `dag_name`, and `run_name`
/// (all non-empty). Isolated home is `capture_dir/dagu-home/`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DaguLocator {
    pub dagu_home: String,
    pub dag_name: String,
    pub run_name: String,
}

/// PATH-resolved `dagu` binary that passed the version gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedDagu {
    pub path: PathBuf,
    pub version: String,
}

/// Resolver or locator failure. Execute messages always name the resolved path
/// (or that PATH lookup found nothing) and required version 2.14.0.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaguError {
    Missing,
    NotRunnable {
        path: PathBuf,
    },
    UnsupportedVersion {
        path: PathBuf,
        found: Option<String>,
    },
    Locator {
        message: String,
    },
}

impl DaguError {
    /// Path named in the error, when a candidate was found.
    #[allow(dead_code)]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Missing | Self::Locator { .. } => None,
            Self::NotRunnable { path } | Self::UnsupportedVersion { path, .. } => Some(path),
        }
    }
}

impl fmt::Display for DaguError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(
                formatter,
                "dagu PATH lookup found nothing; required version {MINIMUM_DAGU_VERSION}"
            ),
            Self::NotRunnable { path } => write!(
                formatter,
                "dagu at `{}` is not a runnable file; required version {MINIMUM_DAGU_VERSION}",
                path.display()
            ),
            Self::UnsupportedVersion {
                path,
                found: Some(found),
            } => write!(
                formatter,
                "dagu at `{}` reports version {found}; required version {MINIMUM_DAGU_VERSION}",
                path.display()
            ),
            Self::UnsupportedVersion { path, found: None } => write!(
                formatter,
                "dagu at `{}` did not report a supported version; required version {MINIMUM_DAGU_VERSION}",
                path.display()
            ),
            Self::Locator { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DaguError {}

impl From<io::Error> for DaguError {
    fn from(error: io::Error) -> Self {
        Self::Locator {
            message: error.to_string(),
        }
    }
}

/// Locate `dagu` on PATH, require a runnable file, and accept only versions
/// `>= 2.14.0` that parse as semver triples (2.14.0 and 2.15.x pass; 2.13.x
/// and non-semver / prefix-mismatch strings fail).
pub fn resolve_dagu() -> Result<PathBuf, DaguError> {
    Ok(resolve_dagu_with_version()?.path)
}

/// Same gate as [`resolve_dagu`], including the parsed version string.
pub fn resolve_dagu_with_version() -> Result<ResolvedDagu, DaguError> {
    let path = locate_dagu()?;
    if !is_runnable(&path) {
        return Err(DaguError::NotRunnable { path });
    }
    let output = Command::new(&path)
        .arg("version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|_| DaguError::NotRunnable { path: path.clone() })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if stdout.trim().is_empty() {
        stderr.as_ref()
    } else {
        stdout.as_ref()
    };
    match parse_supported_version(combined) {
        Some(version) => Ok(ResolvedDagu { path, version }),
        None => {
            let found = first_nonempty_line(combined).map(str::to_owned);
            Err(DaguError::UnsupportedVersion { path, found })
        }
    }
}

/// Isolated home directory: `capture_dir/dagu-home`.
pub fn isolated_home(capture_dir: &Path) -> PathBuf {
    capture_dir.join(HOME_DIR)
}

/// Locator path: `capture_dir/dagu-locator.json`.
pub fn locator_path(capture_dir: &Path) -> PathBuf {
    capture_dir.join(LOCATOR_FILE)
}

/// Unique `dag_name` and `run_name` for this capture dir: `plan-graph-<dir-name>`
/// when that fits Dagu's 39-character graph-name limit; otherwise a stable
/// `plan-graph-<hex>` digest of the absolute capture path so bound invocation
/// directories remain unique and valid.
pub fn names_for_capture_root(capture_dir: &Path) -> Result<(String, String), DaguError> {
    let suffix = capture_dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .ok_or_else(|| DaguError::Locator {
            message: "capture_dir must have a non-empty directory name for dagu locator names"
                .to_owned(),
        })?;
    let name = dagu_graph_name(capture_dir, suffix)?;
    Ok((name.clone(), name))
}

fn dagu_graph_name(capture_dir: &Path, suffix: &str) -> Result<String, DaguError> {
    let candidate = format!("{NAME_PREFIX}-{suffix}");
    if is_dagu_graph_name(&candidate) {
        return Ok(candidate);
    }
    let identity = fs::canonicalize(capture_dir).unwrap_or_else(|_| {
        if capture_dir.is_absolute() {
            capture_dir.to_path_buf()
        } else {
            env::current_dir()
                .map(|cwd| cwd.join(capture_dir))
                .unwrap_or_else(|_| capture_dir.to_path_buf())
        }
    });
    let name = format!("{NAME_PREFIX}-{}", fnv_hex(&identity.to_string_lossy()));
    if !is_dagu_graph_name(&name) {
        return Err(DaguError::Locator {
            message: format!(
                "could not form a Dagu-safe graph name for `{}`",
                capture_dir.display()
            ),
        });
    }
    Ok(name)
}

fn is_dagu_graph_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= DAGU_NAME_MAX
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn fnv_hex(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ac7_9bd9_d547;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Write `capture_dir/dagu-locator.json` and create `capture_dir/dagu-home/`.
///
/// `dagu_home` is the absolute isolated home. `dag_name` and `run_name` must be
/// non-empty; callers should pass [`names_for_capture_root`] so names stay
/// unique per capture directory as `plan-graph-<capture-dir-name>`.
pub fn write_locator(
    capture_dir: &Path,
    dag_name: &str,
    run_name: &str,
) -> Result<DaguLocator, DaguError> {
    if dag_name.is_empty() || run_name.is_empty() {
        return Err(DaguError::Locator {
            message: "dagu locator dag_name and run_name must be non-empty".to_owned(),
        });
    }
    let capture_dir = absolute(capture_dir)?;
    let home = isolated_home(&capture_dir);
    fs::create_dir_all(&home)?;
    let dagu_home = fs::canonicalize(&home)?;
    let dagu_home = path_to_utf8(&dagu_home)?;
    let locator = DaguLocator {
        dagu_home,
        dag_name: dag_name.to_owned(),
        run_name: run_name.to_owned(),
    };
    let encoded = serde_json::to_vec(&locator).map_err(|error| DaguError::Locator {
        message: format!("could not serialize dagu locator: {error}"),
    })?;
    fs::write(locator_path(&capture_dir), encoded)?;
    Ok(locator)
}

fn locate_dagu() -> Result<PathBuf, DaguError> {
    let path_var = env::var_os("PATH").ok_or(DaguError::Missing)?;
    for dir in env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(DAGU_BIN);
        match fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => return absolute(&candidate),
            _ => continue,
        }
    }
    Err(DaguError::Missing)
}

fn is_runnable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn absolute(path: &Path) -> Result<PathBuf, DaguError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn path_to_utf8(path: &Path) -> Result<String, DaguError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DaguError::Locator {
            message: format!("dagu path `{}` is not valid UTF-8", path.display()),
        })
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

/// Accept only `major.minor.patch` integer triples `>= 2.14.0`.
///
/// Prefix mismatch (for example `2.14` matching the start of `2.14.0`, or
/// `2.1` matching `2.14.0`) and non-semver strings fail.
fn parse_supported_version(output: &str) -> Option<String> {
    for token in output.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')');
        let candidate = trimmed.strip_prefix('v').unwrap_or(trimmed);
        if let Some(triple) = parse_version_triple(candidate) {
            if triple >= MINIMUM {
                return Some(format!("{}.{}.{}", triple.0, triple.1, triple.2));
            }
            return None;
        }
    }
    None
}

fn parse_version_triple(token: &str) -> Option<(u64, u64, u64)> {
    let mut parts = token.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if !is_numeric_component(major) || !is_numeric_component(minor) || !is_numeric_component(patch)
    {
        return None;
    }
    Some((
        major.parse().ok()?,
        minor.parse().ok()?,
        patch.parse().ok()?,
    ))
}

fn is_numeric_component(part: &str) -> bool {
    !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_gate_accepts_minimum_and_newer_minor() {
        assert_eq!(parse_supported_version("2.14.0"), Some("2.14.0".to_owned()));
        assert_eq!(
            parse_supported_version("dagu version 2.15.3"),
            Some("2.15.3".to_owned())
        );
        assert_eq!(
            parse_supported_version("v2.14.0"),
            Some("2.14.0".to_owned())
        );
    }

    #[test]
    fn version_gate_rejects_old_prefix_mismatch_and_non_semver() {
        assert_eq!(parse_supported_version("2.13.9"), None);
        assert_eq!(parse_supported_version("2.13.0"), None);
        assert_eq!(parse_supported_version("2.14"), None);
        assert_eq!(parse_supported_version("2.1"), None);
        assert_eq!(parse_supported_version("2.14.0-rc.1"), None);
        assert_eq!(parse_supported_version("development"), None);
        assert_eq!(parse_supported_version(""), None);
    }
}
