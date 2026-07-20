use std::path::{Component, Path};

use thiserror::Error;

use crate::capabilities::{Page, PageCursor, PageRequest};
use crate::model::bounded::{
    BoundError, BoundedText, FILESYSTEM_PATH_UTF8_BYTES, OPAQUE_INTEGRITY_WIRE_UTF8_BYTES,
    PROVIDER_ARGV_ELEMENT_COUNT, PROVIDER_ARGV_ELEMENT_UTF8_BYTES,
    PROVIDER_ARGV_ENCODED_TOTAL_BYTES,
};
use crate::model::ids::{GraphRevision, ProviderHandle, RegistrationId, RunId};
use crate::model::provider::ProviderRegistration;

fn normalize_absolute_path(value: &str) -> Result<String, BoundError> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(BoundError::InvalidType {
            field: "provider_absolute_path",
        });
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::ParentDir => {
                parts.pop();
            }
            Component::Prefix(_) => {
                return Err(BoundError::InvalidType {
                    field: "provider_absolute_path",
                });
            }
        }
    }
    let normalized = if parts.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", parts.join("/"))
    };
    BoundedText::<FILESYSTEM_PATH_UTF8_BYTES>::opaque_non_empty(
        "provider_absolute_path",
        normalized.clone(),
    )?;
    Ok(normalized)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    executable: BoundedText<FILESYSTEM_PATH_UTF8_BYTES>,
    argv: Vec<BoundedText<PROVIDER_ARGV_ELEMENT_UTF8_BYTES>>,
    working_directory: BoundedText<FILESYSTEM_PATH_UTF8_BYTES>,
    timeout_seconds: u64,
}

impl ProviderConfig {
    pub fn new(
        executable: impl Into<String>,
        argv: Vec<String>,
        working_directory: impl Into<String>,
        timeout_seconds: u64,
    ) -> Result<Self, BoundError> {
        let executable = executable.into();
        let working_directory = working_directory.into();
        let executable = normalize_absolute_path(&executable)?;
        let working_directory = normalize_absolute_path(&working_directory)?;
        if timeout_seconds == 0 {
            return Err(BoundError::InvalidType {
                field: "timeout_seconds",
            });
        }
        if argv.len() > PROVIDER_ARGV_ELEMENT_COUNT {
            return Err(BoundError::TooMany {
                field: "provider_argv",
                max: PROVIDER_ARGV_ELEMENT_COUNT,
                actual: argv.len(),
            });
        }
        let argv_bytes = argv.iter().map(String::len).sum::<usize>();
        if argv_bytes > PROVIDER_ARGV_ENCODED_TOTAL_BYTES {
            return Err(BoundError::EncodedTooLarge {
                field: "provider_argv",
                max: PROVIDER_ARGV_ENCODED_TOTAL_BYTES,
                actual: argv_bytes,
            });
        }
        Ok(Self {
            executable: BoundedText::opaque_non_empty("provider_executable", executable)?,
            argv: argv
                .into_iter()
                .map(|value| BoundedText::opaque("provider_argv", value))
                .collect::<Result<_, _>>()?,
            working_directory: BoundedText::opaque_non_empty(
                "provider_working_directory",
                working_directory,
            )?,
            timeout_seconds,
        })
    }

    pub fn executable(&self) -> &str {
        self.executable.as_str()
    }

    pub fn argv(&self) -> &[BoundedText<PROVIDER_ARGV_ELEMENT_UTF8_BYTES>] {
        &self.argv
    }

    pub fn working_directory(&self) -> &str {
        self.working_directory.as_str()
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProviderConfig {
    registration_id: RegistrationId,
    handle: ProviderHandle,
    config_revision: u64,
    config: ProviderConfig,
}

impl ResolvedProviderConfig {
    pub fn new(
        registration_id: RegistrationId,
        handle: ProviderHandle,
        config_revision: u64,
        config: ProviderConfig,
    ) -> Result<Self, BoundError> {
        if config_revision == 0 {
            return Err(BoundError::InvalidType {
                field: "provider_config_revision",
            });
        }
        Ok(Self {
            registration_id,
            handle,
            config_revision,
            config,
        })
    }

    pub fn registration_id(&self) -> &RegistrationId {
        &self.registration_id
    }

    pub fn handle(&self) -> &ProviderHandle {
        &self.handle
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogRow {
    pub registration: ProviderRegistration,
    pub config: Option<ProviderConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderListFilter {
    Enabled,
    Tombstoned,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveRunImpact {
    pub run_id: RunId,
    pub graph_revision: GraphRevision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveSetSnapshot {
    count: u64,
    digest: BoundedText<256>,
    config_revision: u64,
}

impl ActiveSetSnapshot {
    pub fn new(
        count: u64,
        digest: impl Into<String>,
        config_revision: u64,
    ) -> Result<Self, BoundError> {
        if config_revision == 0 {
            return Err(BoundError::InvalidType {
                field: "provider_config_revision",
            });
        }
        Ok(Self {
            count,
            digest: BoundedText::opaque_non_empty("active_set_digest", digest)?,
            config_revision,
        })
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn digest(&self) -> &str {
        self.digest.as_str()
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisableAcknowledgement(BoundedText<OPAQUE_INTEGRITY_WIRE_UTF8_BYTES>);

impl DisableAcknowledgement {
    pub fn parse(value: impl Into<String>) -> Result<Self, BoundError> {
        Ok(Self(BoundedText::opaque_non_empty(
            "disable_acknowledgement",
            value,
        )?))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CatalogCommandError {
    #[error("provider configuration revision must be positive")]
    InvalidConfigRevision,
}

pub fn validate_config_revision(revision: u64) -> Result<(), CatalogCommandError> {
    if revision == 0 {
        Err(CatalogCommandError::InvalidConfigRevision)
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogMutation {
    Add {
        registration_id: RegistrationId,
        handle: ProviderHandle,
        config: ProviderConfig,
    },
    Update {
        registration_id: RegistrationId,
        expected_config_revision: u64,
        config: ProviderConfig,
    },
    Rename {
        registration_id: RegistrationId,
        expected_config_revision: u64,
        handle: ProviderHandle,
    },
    Disable {
        registration_id: RegistrationId,
        expected: ActiveSetSnapshot,
        acknowledgement: DisableAcknowledgement,
    },
    Restore {
        registration_id: RegistrationId,
        expected_config_revision: u64,
        handle: ProviderHandle,
        config: ProviderConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMutationResult {
    pub registration: ProviderRegistration,
    pub affected_active_runs: u64,
    pub impact_cursor: Option<PageCursor>,
}

pub trait ProviderCatalog {
    type Error;

    fn resolve_enabled(
        &self,
        registration_id: &RegistrationId,
    ) -> Result<ResolvedProviderConfig, Self::Error>;

    fn resolve_handle(&self, handle: &ProviderHandle) -> Result<ProviderCatalogRow, Self::Error>;

    /// Stable keyset order; cursor authentication/encoding remains integration-owned.
    fn list(
        &self,
        request: &PageRequest<ProviderListFilter>,
    ) -> Result<Page<ProviderCatalogRow>, Self::Error>;

    /// Stable run-ID keyset order with no row truncation.
    fn active_run_impact(
        &self,
        registration_id: &RegistrationId,
        request: &PageRequest<()>,
    ) -> Result<Page<ActiveRunImpact>, Self::Error>;

    fn active_set_snapshot(
        &self,
        registration_id: &RegistrationId,
    ) -> Result<ActiveSetSnapshot, Self::Error>;

    fn mutate(&self, command: CatalogMutation) -> Result<CatalogMutationResult, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::{ActiveSetSnapshot, ProviderConfig, ResolvedProviderConfig};
    use crate::model::ids::{ProviderHandle, RegistrationId};

    #[test]
    fn provider_configuration_is_explicit_bounded_and_revisioned() {
        assert!(ProviderConfig::new("relative", vec![], "/work", 1).is_err());
        assert!(ProviderConfig::new("/provider", vec![], "relative", 1).is_err());
        assert!(ProviderConfig::new("/provider", vec![], "/work", 0).is_err());
        assert!(ProviderConfig::new("/provider", vec!["x".into(); 129], "/work", 1).is_err());
        let config = ProviderConfig::new(
            "/opt/../provider",
            vec!["--flag".into()],
            "/work/./tree/..",
            60,
        )
        .unwrap();
        assert_eq!(config.executable(), "/provider");
        assert_eq!(config.working_directory(), "/work");
        assert!(
            ResolvedProviderConfig::new(
                RegistrationId::parse("registration").unwrap(),
                ProviderHandle::parse("provider").unwrap(),
                0,
                config.clone(),
            )
            .is_err()
        );
        let resolved = ResolvedProviderConfig::new(
            RegistrationId::parse("registration").unwrap(),
            ProviderHandle::parse("provider").unwrap(),
            2,
            config,
        )
        .unwrap();
        assert_eq!(resolved.config_revision(), 2);
        assert_eq!(resolved.config().timeout_seconds(), 60);
        assert!(ActiveSetSnapshot::new(0, "digest", 0).is_err());
    }
}
