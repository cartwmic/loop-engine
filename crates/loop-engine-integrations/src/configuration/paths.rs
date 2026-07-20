use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use loop_engine_core::model::bounded::FILESYSTEM_PATH_UTF8_BYTES;

use super::error::ConfigurationError;

#[cfg(target_os = "linux")]
const SYMLINK_LOOP_ERRNO: i32 = 40;
#[cfg(target_os = "macos")]
const SYMLINK_LOOP_ERRNO: i32 = 62;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachinePaths {
    pub machine_home_root: PathBuf,
    pub config_root: PathBuf,
    pub state_root: PathBuf,
    pub global_config: PathBuf,
    pub database: PathBuf,
    pub traces: PathBuf,
}

impl MachinePaths {
    pub fn from_environment() -> Result<Self, ConfigurationError> {
        let environment = EnvironmentPaths {
            home: std::env::var_os("HOME"),
            loop_engine_home: std::env::var_os("LOOP_ENGINE_HOME"),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME"),
            xdg_state_home: std::env::var_os("XDG_STATE_HOME"),
        };
        Self::resolve(&environment)
    }

    pub fn resolve(environment: &EnvironmentPaths) -> Result<Self, ConfigurationError> {
        if let Some(override_home) = environment
            .loop_engine_home
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            let lexical = lexical_absolute(override_home, environment.home.as_deref())?;
            let machine_home_root = resolve_final_symlinks(lexical)?;
            return Self::from_roots(machine_home_root.clone(), machine_home_root);
        }

        #[cfg(target_os = "macos")]
        let (config_root, state_root) = {
            let home = environment
                .home
                .as_deref()
                .filter(|value| !value.is_empty())
                .ok_or(ConfigurationError::HomeUnavailable)?;
            let home = lexical_absolute(home, None)?;
            let root = home.join("Library/Application Support/loop-engine");
            (root.clone(), root)
        };
        #[cfg(target_os = "linux")]
        let (config_root, state_root) = {
            let home = environment
                .home
                .as_deref()
                .filter(|value| !value.is_empty());
            let config_base = match environment
                .xdg_config_home
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                Some(value) => lexical_absolute(value, home)?,
                None => lexical_absolute(home.ok_or(ConfigurationError::HomeUnavailable)?, None)?
                    .join(".config"),
            };
            let state_base = match environment
                .xdg_state_home
                .as_deref()
                .filter(|value| !value.is_empty())
            {
                Some(value) => lexical_absolute(value, home)?,
                None => lexical_absolute(home.ok_or(ConfigurationError::HomeUnavailable)?, None)?
                    .join(".local/state"),
            };
            (
                config_base.join("loop-engine"),
                state_base.join("loop-engine"),
            )
        };
        Self::from_roots(config_root, state_root)
    }

    fn from_roots(config_root: PathBuf, state_root: PathBuf) -> Result<Self, ConfigurationError> {
        let paths = Self {
            machine_home_root: state_root.clone(),
            global_config: config_root.join("config.toml"),
            database: state_root.join("state.db"),
            traces: state_root.join("traces"),
            config_root,
            state_root,
        };
        for path in [
            &paths.machine_home_root,
            &paths.global_config,
            &paths.database,
            &paths.traces,
            &paths.config_root,
            &paths.state_root,
        ] {
            validate_utf8_path(path)?;
        }
        Ok(paths)
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvironmentPaths {
    pub home: Option<OsString>,
    pub loop_engine_home: Option<OsString>,
    pub xdg_config_home: Option<OsString>,
    pub xdg_state_home: Option<OsString>,
}

pub fn discover_project_config(cwd: &Path) -> Result<Option<PathBuf>, ConfigurationError> {
    if !cwd.is_absolute() {
        return Err(ConfigurationError::RelativePath(
            cwd.to_string_lossy().into_owned(),
        ));
    }
    let mut directory = cwd;
    loop {
        let candidate = directory.join(".loop-engine.toml");
        match std::fs::metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => return Ok(Some(candidate)),
            Ok(_) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(SYMLINK_LOOP_ERRNO) => {}
            Err(error) => {
                return Err(ConfigurationError::Inspect {
                    path: candidate,
                    source: error,
                });
            }
        }
        let Some(parent) = directory.parent() else {
            return Ok(None);
        };
        if parent == directory {
            return Ok(None);
        }
        directory = parent;
    }
}

fn validate_utf8_path(path: &Path) -> Result<(), ConfigurationError> {
    let bytes = path.as_os_str().as_bytes();
    std::str::from_utf8(bytes).map_err(|_| ConfigurationError::PathNotUtf8)?;
    if bytes.len() > FILESYSTEM_PATH_UTF8_BYTES {
        return Err(ConfigurationError::PathTooLong {
            max: FILESYSTEM_PATH_UTF8_BYTES,
            actual: bytes.len(),
        });
    }
    Ok(())
}

fn lexical_absolute(value: &OsStr, home: Option<&OsStr>) -> Result<PathBuf, ConfigurationError> {
    let text =
        std::str::from_utf8(value.as_bytes()).map_err(|_| ConfigurationError::PathNotUtf8)?;
    let expanded = if text == "~" {
        PathBuf::from(home.ok_or(ConfigurationError::HomeUnavailable)?)
    } else if let Some(remainder) = text.strip_prefix("~/") {
        Path::new(home.ok_or(ConfigurationError::HomeUnavailable)?).join(remainder)
    } else {
        PathBuf::from(value)
    };
    let expanded_bytes = expanded.as_os_str().as_bytes();
    std::str::from_utf8(expanded_bytes).map_err(|_| ConfigurationError::PathNotUtf8)?;
    if expanded_bytes.len() > FILESYSTEM_PATH_UTF8_BYTES {
        return Err(ConfigurationError::PathTooLong {
            max: FILESYSTEM_PATH_UTF8_BYTES,
            actual: expanded_bytes.len(),
        });
    }
    if !expanded.is_absolute() {
        return Err(ConfigurationError::RelativePath(text.to_owned()));
    }
    let mut output = PathBuf::from("/");
    for component in expanded.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            Component::Normal(value) => output.push(value),
            Component::Prefix(_) => unreachable!("Unix-only path contract"),
        }
    }
    Ok(output)
}

fn resolve_final_symlinks(mut path: PathBuf) -> Result<PathBuf, ConfigurationError> {
    let mut visited = BTreeSet::new();
    loop {
        if !path.exists() {
            return Ok(path);
        }
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|source| ConfigurationError::Inspect {
                path: path.clone(),
                source,
            })?;
        if !metadata.file_type().is_symlink() {
            return Ok(path);
        }
        if !visited.insert(path.clone()) {
            return Err(ConfigurationError::Inspect {
                path: path.clone(),
                source: std::io::Error::from_raw_os_error(SYMLINK_LOOP_ERRNO),
            });
        }
        let target = std::fs::read_link(&path).map_err(|source| ConfigurationError::Inspect {
            path: path.clone(),
            source,
        })?;
        let target = if target.is_absolute() {
            target
        } else {
            path.parent().unwrap_or(Path::new("/")).join(target)
        };
        path = lexical_absolute(target.as_os_str(), None)?;
    }
}
