//! Provider fixture executable and registration config helpers (T145).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAddArgs {
    pub handle: String,
    pub exec: PathBuf,
    pub working_directory: PathBuf,
    pub args: Vec<String>,
    pub timeout_seconds: u64,
}

impl ProviderAddArgs {
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut argv = vec![
            "provider".into(),
            "add".into(),
            self.handle.clone(),
            "--exec".into(),
            self.exec.display().to_string(),
            "--working-directory".into(),
            self.working_directory.display().to_string(),
        ];
        for arg in &self.args {
            argv.push("--arg".into());
            argv.push(arg.clone());
        }
        argv.push("--timeout".into());
        argv.push(self.timeout_seconds.to_string());
        argv
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfigFile {
    pub path: PathBuf,
    pub value: Value,
}

impl ProviderConfigFile {
    pub fn scenario(
        path: PathBuf,
        handle: &str,
        executable: &Path,
        working_directory: &Path,
        scenario: &str,
        timeout_seconds: u64,
    ) -> Self {
        Self {
            path,
            value: json!({
                "handle": handle,
                "executable": executable.display().to_string(),
                "working_directory": working_directory.display().to_string(),
                "argv": ["--scenario", scenario],
                "timeout_seconds": timeout_seconds,
            }),
        }
    }

    pub fn write(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &self.path,
            serde_json::to_string_pretty(&self.value).expect("provider config serializes"),
        )
    }

    pub fn provider_add_args(&self, exec_override: Option<&Path>) -> ProviderAddArgs {
        let exec = exec_override
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(self.value["executable"].as_str().expect("exec")));
        ProviderAddArgs {
            handle: self.value["handle"].as_str().expect("handle").to_owned(),
            exec,
            working_directory: PathBuf::from(
                self.value["working_directory"]
                    .as_str()
                    .expect("working_directory"),
            ),
            args: self.value["argv"]
                .as_array()
                .expect("argv")
                .iter()
                .map(|value| value.as_str().expect("argv element").to_owned())
                .collect(),
            timeout_seconds: self.value["timeout_seconds"]
                .as_u64()
                .expect("timeout_seconds"),
        }
    }
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn provider_manifest_path(package: &str) -> PathBuf {
    workspace_root()
        .join("test-support/providers")
        .join(package)
        .join("Cargo.toml")
}

pub fn scenario_provider_manifest_path() -> PathBuf {
    provider_manifest_path("scenario-provider")
}

pub fn reference_provider_manifest_path() -> PathBuf {
    provider_manifest_path("reference-provider")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderExecutableError {
    BinaryNotBuilt {
        package: String,
        binary: String,
        expected: PathBuf,
        manifest: PathBuf,
    },
}

impl std::fmt::Display for ProviderExecutableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinaryNotBuilt {
                package,
                binary,
                expected,
                manifest,
            } => write!(
                formatter,
                "provider executable for package {package} binary {binary} is not built at {} (run `cargo build --manifest-path {} --locked`)",
                expected.display(),
                manifest.display()
            ),
        }
    }
}

impl std::error::Error for ProviderExecutableError {}

/// Cargo's target-directory rule for standalone `--manifest-path` builds.
///
/// When `cargo_target_dir` is unset: `<manifest-parent>/target`.
/// When absolute: the path is used as-is.
/// When relative: `<invocation_cwd>/<cargo_target_dir>` (Cargo resolves relative
/// `CARGO_TARGET_DIR` from the process current working directory at invocation).
pub(crate) fn resolve_provider_target_dir(
    manifest: &Path,
    cargo_target_dir: Option<&Path>,
    invocation_cwd: &Path,
) -> PathBuf {
    match cargo_target_dir {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => invocation_cwd.join(path),
        None => manifest
            .parent()
            .expect("provider manifest has parent")
            .join("target"),
    }
}

pub(crate) fn resolve_provider_executable_debug_dir(
    manifest: &Path,
    cargo_target_dir: Option<&Path>,
    invocation_cwd: &Path,
) -> PathBuf {
    resolve_provider_target_dir(manifest, cargo_target_dir, invocation_cwd).join("debug")
}

pub(crate) fn resolve_provider_executable_candidate(
    manifest: &Path,
    binary_name: &str,
    cargo_target_dir: Option<&Path>,
    invocation_cwd: &Path,
) -> PathBuf {
    resolve_provider_executable_debug_dir(manifest, cargo_target_dir, invocation_cwd)
        .join(format!("{binary_name}{}", std::env::consts::EXE_SUFFIX))
}

fn provider_invocation_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| workspace_root())
}

fn runtime_cargo_target_dir() -> Option<PathBuf> {
    std::env::var("CARGO_TARGET_DIR").ok().map(PathBuf::from)
}

pub fn resolve_provider_executable_path(
    package: &str,
    binary_name: &str,
) -> Result<PathBuf, ProviderExecutableError> {
    let manifest = provider_manifest_path(package);
    let invocation_cwd = provider_invocation_cwd();
    let cargo_target_dir = runtime_cargo_target_dir();
    let expected = resolve_provider_executable_candidate(
        &manifest,
        binary_name,
        cargo_target_dir.as_deref(),
        &invocation_cwd,
    );
    if expected.is_file() {
        Ok(expected.canonicalize().unwrap_or(expected))
    } else {
        Err(ProviderExecutableError::BinaryNotBuilt {
            package: package.to_owned(),
            binary: binary_name.to_owned(),
            expected,
            manifest,
        })
    }
}

#[cfg(test)]
mod provider_target_resolution_tests {
    use super::*;

    #[test]
    fn unset_target_dir_uses_package_local_target() {
        let manifest = provider_manifest_path("scenario-provider");
        let invocation_cwd = PathBuf::from("/tmp/invocation-cwd");

        assert_eq!(
            resolve_provider_target_dir(&manifest, None, &invocation_cwd),
            manifest.parent().expect("manifest parent").join("target")
        );
    }

    #[test]
    fn absolute_target_dir_is_used_as_is() {
        let manifest = provider_manifest_path("scenario-provider");
        let absolute = PathBuf::from("/tmp/custom-target");
        let invocation_cwd = PathBuf::from("/tmp/invocation-cwd");

        assert_eq!(
            resolve_provider_target_dir(&manifest, Some(&absolute), &invocation_cwd),
            absolute
        );
    }

    #[test]
    fn relative_target_dir_resolves_against_invocation_cwd() {
        let manifest = provider_manifest_path("scenario-provider");
        let invocation_cwd = workspace_root();
        let relative = Path::new("wp2-relative-target-probe");

        assert_eq!(
            resolve_provider_target_dir(&manifest, Some(relative), &invocation_cwd),
            invocation_cwd.join("wp2-relative-target-probe")
        );
    }

    #[test]
    fn executable_candidate_uses_debug_dir_and_exe_suffix() {
        let manifest = provider_manifest_path("scenario-provider");
        let invocation_cwd = PathBuf::from("/tmp/invocation-cwd");
        let binary = "scenario-provider";

        assert_eq!(
            resolve_provider_executable_candidate(&manifest, binary, None, &invocation_cwd),
            manifest
                .parent()
                .expect("manifest parent")
                .join("target")
                .join("debug")
                .join(format!("{binary}{}", std::env::consts::EXE_SUFFIX))
        );
    }
}
