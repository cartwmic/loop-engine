//! Local TOML provider configuration and durable subprocess associations.
//!
//! Provider aliases are a configuration-time convenience only.  A resolver
//! turns an alias into the command and arguments that were configured for it;
//! the resulting [`loop_core::ProviderAssociation`] contains no alias and can
//! therefore be stored on a run and used without reading this configuration
//! again.

use loop_core::{ProviderAssociation, ProviderResolutionError, ProviderResolver, ProviderSelector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, fmt, fs, path::Path};

/// Parsed local provider configuration.
///
/// The corresponding TOML has the following shape:
///
/// ```toml
/// [providers.software-change]
/// command = "/path/to/provider"
/// args = ["--example"]
/// ```
///
/// Alias lookup is exact and case-sensitive.  The map is copied into a
/// resolver, so changing a separate configuration value cannot change an
/// association that was already resolved.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ProviderConfiguration {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderDefinition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProviderConfiguration {
    #[serde(default)]
    providers: BTreeMap<String, ProviderDefinition>,
}

impl<'de> Deserialize<'de> for ProviderConfiguration {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawProviderConfiguration::deserialize(deserializer)?;
        let configuration = Self {
            providers: raw.providers,
        };
        configuration.validate().map_err(serde::de::Error::custom)?;
        Ok(configuration)
    }
}

/// A single configured provider invocation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDefinition {
    /// The command path (or command name) passed to the eventual subprocess
    /// gateway.  Resolution deliberately does not execute or canonicalize it.
    pub command: String,
    /// Arguments retained in their configured order.
    #[serde(default)]
    pub args: Vec<String>,
}

/// The invocation identity encoded in a durable provider association.
///
/// This has the same JSON shape as a resolved association:
///
/// ```json
/// {"command":"/path/to/provider","args":["--example"]}
/// ```
///
/// It intentionally contains neither the mutable alias nor a configuration
/// path.  Updating an alias later consequently cannot redirect an existing
/// run.  Executable contents may still change at the stored command path, as
/// allowed by the v0.1 provider contract.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderInvocation {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// A resolver backed by parsed local TOML configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfiguredProviderResolver {
    configuration: ProviderConfiguration,
}

/// Descriptive error returned when a stored association cannot be decoded by
/// the subprocess integration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderAssociationError {
    message: String,
}

impl ProviderAssociationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProviderAssociationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderAssociationError {}

impl ProviderDefinition {
    /// Construct one configured provider entry.
    pub fn new(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Construct an entry with no command-line arguments.
    pub fn command(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
        }
    }

    fn validate(&self, alias: &str) -> Result<(), ProviderResolutionError> {
        if alias.trim().is_empty() {
            return Err(ProviderResolutionError::invalid_configuration(
                "empty-provider-alias",
                "provider aliases must not be empty",
            ));
        }
        if self.command.is_empty() {
            return Err(ProviderResolutionError::invalid_configuration(
                "empty-provider-command",
                format!("provider alias `{alias}` has an empty command"),
            ));
        }
        Ok(())
    }

    fn invocation(&self, alias: &str) -> Result<ProviderInvocation, ProviderResolutionError> {
        self.validate(alias)?;
        Ok(ProviderInvocation {
            command: self.command.clone(),
            args: self.args.clone(),
        })
    }
}

impl ProviderInvocation {
    pub fn new(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    /// Convert the resolved invocation into the opaque core association that
    /// persistence stores with a run.
    pub fn into_association(self) -> ProviderAssociation {
        ProviderAssociation::new(json!({
            "command": self.command,
            "args": self.args,
        }))
    }

    /// Borrowing counterpart to [`Self::into_association`].
    pub fn to_association(&self) -> ProviderAssociation {
        ProviderAssociation::new(json!({
            "command": self.command,
            "args": self.args,
        }))
    }

    /// Decode an association retained by a run without consulting aliases or
    /// the current TOML source.
    pub fn from_association(
        association: &ProviderAssociation,
    ) -> Result<Self, ProviderAssociationError> {
        let value = association.as_json().clone();
        let invocation: Self = serde_json::from_value(value).map_err(|error| {
            ProviderAssociationError::new(format!("invalid provider association: {error}"))
        })?;
        if invocation.command.is_empty() {
            return Err(ProviderAssociationError::new(
                "invalid provider association: command is empty",
            ));
        }
        Ok(invocation)
    }
}

impl TryFrom<&ProviderAssociation> for ProviderInvocation {
    type Error = ProviderAssociationError;

    fn try_from(value: &ProviderAssociation) -> Result<Self, Self::Error> {
        Self::from_association(value)
    }
}

impl ProviderConfiguration {
    /// Parse a TOML configuration source.
    pub fn parse(source: &str) -> Result<Self, ProviderResolutionError> {
        toml::from_str(source).map_err(|error| {
            ProviderResolutionError::invalid_configuration(
                "invalid-toml",
                format!("could not parse provider configuration: {error}"),
            )
        })
    }

    /// Read and parse a TOML configuration file.
    ///
    /// Reading failures are reported as `Unavailable`; syntactically invalid
    /// or semantically invalid contents are reported as
    /// `InvalidConfiguration`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ProviderResolutionError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|error| {
            ProviderResolutionError::unavailable(
                "provider-configuration-unavailable",
                format!(
                    "could not read provider configuration `{}`: {error}",
                    path.display()
                ),
            )
        })?;
        Self::parse(&source)
    }

    /// Return all aliases in deterministic lexical order.
    pub fn providers(&self) -> &BTreeMap<String, ProviderDefinition> {
        &self.providers
    }

    /// Look up one alias without resolving it.
    pub fn get(&self, selector: &ProviderSelector) -> Option<&ProviderDefinition> {
        self.providers.get(selector.as_str())
    }

    /// Resolve directly from this configuration.
    pub fn resolve(
        &self,
        selector: &ProviderSelector,
    ) -> Result<ProviderAssociation, ProviderResolutionError> {
        ConfiguredProviderResolver::new(self.clone()).resolve(selector)
    }

    fn validate(&self) -> Result<(), ProviderResolutionError> {
        for (alias, definition) in &self.providers {
            definition.validate(alias)?;
        }
        Ok(())
    }
}

impl std::str::FromStr for ProviderConfiguration {
    type Err = ProviderResolutionError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::parse(source)
    }
}

impl ConfiguredProviderResolver {
    /// Create a resolver from already parsed configuration.
    pub fn new(configuration: ProviderConfiguration) -> Self {
        Self { configuration }
    }

    pub fn configuration(&self) -> &ProviderConfiguration {
        &self.configuration
    }

    /// Resolve an alias without requiring the core trait to be imported at
    /// the call site.  The trait implementation below delegates here.
    pub fn resolve(
        &self,
        selector: &ProviderSelector,
    ) -> Result<ProviderAssociation, ProviderResolutionError> {
        let definition = self
            .configuration
            .providers
            .get(selector.as_str())
            .ok_or_else(|| ProviderResolutionError::UnknownSelector {
                selector: selector.clone(),
            })?;
        let invocation = definition.invocation(selector.as_str())?;
        Ok(invocation.into_association())
    }
}

impl ProviderResolver for ProviderConfiguration {
    fn resolve(
        &self,
        selector: &ProviderSelector,
    ) -> Result<ProviderAssociation, ProviderResolutionError> {
        Self::resolve(self, selector)
    }
}

impl ProviderResolver for ConfiguredProviderResolver {
    fn resolve(
        &self,
        selector: &ProviderSelector,
    ) -> Result<ProviderAssociation, ProviderResolutionError> {
        Self::resolve(self, selector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const VALID: &str = r#"
        [providers.software-change]
        command = "/usr/local/bin/software-change"
        args = ["--strict", "two words"]
    "#;

    #[test]
    fn parses_valid_alias_configuration() {
        let configuration = ProviderConfiguration::parse(VALID).expect("valid TOML");
        let definition = configuration
            .providers()
            .get("software-change")
            .expect("alias");
        assert_eq!(definition.command, "/usr/local/bin/software-change");
        assert_eq!(definition.args, ["--strict", "two words"]);
    }

    #[test]
    fn rejects_invalid_toml_and_invalid_entries() {
        let error = ProviderConfiguration::parse("[providers.foo").unwrap_err();
        assert!(matches!(
            error,
            ProviderResolutionError::InvalidConfiguration { .. }
        ));

        let error = ProviderConfiguration::parse(
            r#"
                [providers.foo]
                command = ""
            "#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ProviderResolutionError::InvalidConfiguration { .. }
        ));

        let error = ProviderConfiguration::parse(
            r#"
                [providers.foo]
                command = "/bin/foo"
                args = [42]
            "#,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ProviderResolutionError::InvalidConfiguration { .. }
        ));
    }

    #[test]
    fn resolves_selector_to_invocation_association() {
        let resolver = ConfiguredProviderResolver::new(
            ProviderConfiguration::parse(VALID).expect("valid config"),
        );
        let association = resolver
            .resolve(&ProviderSelector::from("software-change"))
            .expect("known alias");
        assert_eq!(
            association.as_json(),
            &json!({
                "command": "/usr/local/bin/software-change",
                "args": ["--strict", "two words"]
            })
        );
    }

    #[test]
    fn unknown_selector_is_reported_without_fallback() {
        let resolver = ConfiguredProviderResolver::new(
            ProviderConfiguration::parse(VALID).expect("valid config"),
        );
        let error = resolver
            .resolve(&ProviderSelector::from("missing"))
            .unwrap_err();
        assert_eq!(
            error,
            ProviderResolutionError::UnknownSelector {
                selector: ProviderSelector::from("missing")
            }
        );
    }

    #[test]
    fn association_round_trip_decodes_without_configuration() {
        let resolver = ConfiguredProviderResolver::new(
            ProviderConfiguration::parse(VALID).expect("valid config"),
        );
        let association = resolver
            .resolve(&ProviderSelector::from("software-change"))
            .expect("known alias");
        let serialized = serde_json::to_string(&association).expect("serialize association");
        let restored: ProviderAssociation =
            serde_json::from_str(&serialized).expect("deserialize association");
        let invocation = ProviderInvocation::from_association(&restored).expect("invocation");
        assert_eq!(invocation.command, "/usr/local/bin/software-change");
        assert_eq!(invocation.args, ["--strict", "two words"]);
    }

    #[test]
    fn changing_alias_does_not_change_existing_association() {
        let old_resolver = ConfiguredProviderResolver::new(
            ProviderConfiguration::parse(
                r#"
                    [providers.software-change]
                    command = "/old/provider"
                    args = ["--old"]
                "#,
            )
            .expect("old config"),
        );
        let association = old_resolver
            .resolve(&ProviderSelector::from("software-change"))
            .expect("known alias");

        let new_resolver = ConfiguredProviderResolver::new(
            ProviderConfiguration::parse(
                r#"
                    [providers.software-change]
                    command = "/new/provider"
                    args = ["--new"]
                "#,
            )
            .expect("new config"),
        );
        let new_association = new_resolver
            .resolve(&ProviderSelector::from("software-change"))
            .expect("known alias");

        assert_eq!(
            ProviderInvocation::from_association(&association)
                .expect("old association")
                .command,
            "/old/provider"
        );
        assert_eq!(
            ProviderInvocation::from_association(&new_association)
                .expect("new association")
                .command,
            "/new/provider"
        );
    }
}
