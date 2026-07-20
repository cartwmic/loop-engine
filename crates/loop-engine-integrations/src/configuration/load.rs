use std::io::Read;
use std::path::Path;

use super::dto::ConfigurationDto;
use super::error::ConfigurationError;

pub const TOML_CONFIG_FILE_BYTES: usize = 1_048_576;

pub fn load_optional(path: &Path) -> Result<Option<ConfigurationDto>, ConfigurationError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigurationError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let metadata_length = file
        .metadata()
        .map_err(|source| ConfigurationError::Read {
            path: path.to_owned(),
            source,
        })?
        .len();
    if metadata_length > TOML_CONFIG_FILE_BYTES as u64 {
        return Err(ConfigurationError::TooLarge {
            path: path.to_owned(),
            max: TOML_CONFIG_FILE_BYTES,
            actual: usize::try_from(metadata_length).unwrap_or(usize::MAX),
        });
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata_length)
            .unwrap_or(TOML_CONFIG_FILE_BYTES)
            .min(TOML_CONFIG_FILE_BYTES),
    );
    file.take(TOML_CONFIG_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ConfigurationError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > TOML_CONFIG_FILE_BYTES {
        return Err(ConfigurationError::TooLarge {
            path: path.to_owned(),
            max: TOML_CONFIG_FILE_BYTES,
            actual: bytes.len(),
        });
    }
    let text = std::str::from_utf8(&bytes).map_err(|error| ConfigurationError::Malformed {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    let dto: ConfigurationDto =
        toml::from_str(text).map_err(|error| ConfigurationError::Malformed {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
    if dto.schema_version != 1 {
        return Err(ConfigurationError::UnsupportedVersion {
            path: path.to_owned(),
            actual: dto.schema_version,
        });
    }
    if dto.defaults.timeout_seconds == Some(0) {
        return Err(ConfigurationError::NonPositiveTimeout);
    }
    Ok(Some(dto))
}
