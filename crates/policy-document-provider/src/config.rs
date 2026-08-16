use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InitialInput {
    pub schema_version: u32,
    pub profile_version: String,
    pub mode: Mode,
    pub target: Target,
    pub deterministic_policies: Vec<DeterministicPolicy>,
    pub semantic_policies: Vec<SemanticPolicy>,
    /// Reserved catalog key composed at start. Accepted and ignored; never a file root.
    #[allow(dead_code)]
    #[serde(default)]
    pub artifact_root: Option<String>,
    /// Reserved engine key for frozen slot CLI bindings. Accepted and ignored.
    #[allow(dead_code)]
    #[serde(default)]
    pub work_slot_bindings: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Draft,
    Audit,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub id: String,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum DeterministicPolicy {
    #[serde(rename = "non-empty")]
    NonEmpty { id: String },
    #[serde(rename = "required-heading")]
    RequiredHeading {
        id: String,
        aliases: Vec<String>,
        level: Option<u8>,
    },
    #[serde(rename = "any-heading")]
    AnyHeading { id: String, level: u8 },
    #[serde(rename = "command-in-section")]
    CommandInSection {
        id: String,
        section_aliases: Vec<String>,
    },
    #[serde(rename = "local-references-resolve")]
    LocalReferencesResolve { id: String },
}

impl DeterministicPolicy {
    pub fn id(&self) -> &str {
        match self {
            Self::NonEmpty { id }
            | Self::RequiredHeading { id, .. }
            | Self::AnyHeading { id, .. }
            | Self::CommandInSection { id, .. }
            | Self::LocalReferencesResolve { id } => id,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPolicy {
    pub id: String,
    pub description: String,
    pub example_prompt: String,
}

impl InitialInput {
    pub fn parse(value: &Value) -> Result<Self, String> {
        let input: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("invalid policy-document initial input: {e}"))?;
        if input.schema_version != 1 {
            return Err("schema_version must be 1".into());
        }
        nonempty("profile_version", &input.profile_version)?;
        nonempty("target.id", &input.target.id)?;
        nonempty("target.path", &input.target.path)?;
        if !Path::new(&input.target.path).is_absolute() {
            return Err("target.path must be absolute".into());
        }
        if input.deterministic_policies.is_empty() {
            return Err("deterministic_policies must be non-empty".into());
        }
        if input.semantic_policies.is_empty() {
            return Err("semantic_policies must be non-empty".into());
        }
        unique(
            input
                .deterministic_policies
                .iter()
                .map(DeterministicPolicy::id),
            "deterministic policy",
        )?;
        unique(
            input.semantic_policies.iter().map(|p| p.id.as_str()),
            "semantic policy",
        )?;
        for policy in &input.deterministic_policies {
            validate_deterministic(policy)?;
        }
        for policy in &input.semantic_policies {
            nonempty("semantic policy id", &policy.id)?;
            nonempty("semantic policy description", &policy.description)?;
            nonempty("semantic policy example_prompt", &policy.example_prompt)?;
        }
        Ok(input)
    }
}

fn nonempty(name: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{name} must be non-empty"))
    } else {
        Ok(())
    }
}
fn unique<'a>(values: impl Iterator<Item = &'a str>, label: &str) -> Result<(), String> {
    let mut seen = HashSet::new();
    for value in values {
        nonempty(label, value)?;
        if !seen.insert(value) {
            return Err(format!("duplicate {label} id `{value}`"));
        }
    }
    Ok(())
}
fn aliases(values: &[String], label: &str) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!("{label} must be non-empty"));
    }
    for value in values {
        nonempty(label, value)?;
    }
    Ok(())
}
fn validate_deterministic(policy: &DeterministicPolicy) -> Result<(), String> {
    match policy {
        DeterministicPolicy::NonEmpty { .. }
        | DeterministicPolicy::LocalReferencesResolve { .. } => Ok(()),
        DeterministicPolicy::RequiredHeading {
            aliases: values,
            level,
            ..
        } => {
            aliases(values, "aliases")?;
            if let Some(level) = level {
                if !(1..=6).contains(level) {
                    return Err("heading level must be 1..6".into());
                }
            }
            Ok(())
        }
        DeterministicPolicy::AnyHeading { level, .. } => {
            if (1..=6).contains(level) {
                Ok(())
            } else {
                Err("heading level must be 1..6".into())
            }
        }
        DeterministicPolicy::CommandInSection {
            section_aliases, ..
        } => aliases(section_aliases, "section_aliases"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shipped_profiles_have_frozen_policy_sets() {
        for (name, version, target, count, semantic) in [
            ("readme", "readme-2", "README.md", 9, 7),
            ("agents", "agents-2", "AGENTS.md", 6, 9),
        ] {
            let raw = if name == "readme" {
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/readme.json"))
            } else {
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/agents.json"))
            };
            let mut value: Value = serde_json::from_str(raw).unwrap();
            value["target"]["path"] = Value::String(format!("/tmp/{target}"));
            let parsed = InitialInput::parse(&value).unwrap();
            assert_eq!(parsed.profile_version, version);
            assert_eq!(parsed.target.id, target);
            assert_eq!(parsed.deterministic_policies.len(), count);
            assert_eq!(parsed.semantic_policies.len(), semantic);
            let ids = parsed
                .deterministic_policies
                .iter()
                .map(DeterministicPolicy::id)
                .collect::<Vec<_>>();
            let expected = if name == "readme" {
                vec![
                    "document-present",
                    "project-title",
                    "purpose",
                    "onboarding",
                    "usage",
                    "validation",
                    "onboarding-command",
                    "validation-command",
                    "local-references",
                ]
            } else {
                vec![
                    "document-present",
                    "scope-authority",
                    "workflow-validation",
                    "completion-handoff",
                    "workflow-command",
                    "local-references",
                ]
            };
            assert_eq!(ids, expected);
            let semantic_ids = parsed
                .semantic_policies
                .iter()
                .map(|policy| policy.id.as_str())
                .collect::<Vec<_>>();
            let expected_semantic = if name == "readme" {
                vec![
                    "product-fidelity",
                    "onboarding-sufficiency",
                    "audience-navigation",
                    "clarity-scope",
                    "honest-fitness",
                    "verifiable-claims",
                    "troubleshooting-sharp-edges",
                ]
            } else {
                vec![
                    "success-path-completeness",
                    "operational-precision",
                    "authority-resolution",
                    "risk-boundary-sufficiency",
                    "completion-handoff",
                    "non-discoverable-sharp-edges",
                    "ambiguity-resolution",
                    "signal-density",
                    "living-config",
                ]
            };
            assert_eq!(semantic_ids, expected_semantic);
            assert!(!ids.contains(&"project-title") || name == "readme");
            assert!(matches!(
                parsed.deterministic_policies.last(),
                Some(DeterministicPolicy::LocalReferencesResolve { .. })
            ));
            assert!(parsed
                .deterministic_policies
                .iter()
                .any(|policy| matches!(policy, DeterministicPolicy::CommandInSection { .. })));
        }
    }
    #[test]
    fn closed_config_rejects_unknown_and_empty_collections() {
        let mut value: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/readme.json"
        )))
        .unwrap();
        value["target"]["path"] = Value::String("/tmp/README.md".into());
        let mut with_root = value.clone();
        with_root["artifact_root"] = Value::String("/tmp/unused".into());
        assert!(InitialInput::parse(&with_root).is_ok());
        let mut with_null = value.clone();
        with_null["artifact_root"] = Value::Null;
        assert!(InitialInput::parse(&with_null).is_ok());
        let mut unknown = value.clone();
        unknown["unknown"] = Value::Bool(true);
        assert!(InitialInput::parse(&unknown).is_err());
        let mut empty = value.clone();
        empty["deterministic_policies"] = Value::Array(Vec::new());
        assert!(InitialInput::parse(&empty).is_err());
    }

    #[test]
    fn initial_input_accepts_reserved_artifact_root_and_ignores_it() {
        let mut value = base();
        value["artifact_root"] = Value::String("/tmp/unused".into());
        let parsed = InitialInput::parse(&value).unwrap();
        assert_eq!(parsed.artifact_root.as_deref(), Some("/tmp/unused"));
        assert_eq!(parsed.target.path, "/tmp/README.md");
        value["not_a_reserved_key"] = Value::Bool(true);
        let err = InitialInput::parse(&value).unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
    }

    #[test]
    fn initial_input_accepts_reserved_work_slot_bindings_and_ignores_them() {
        let mut value = base();
        value["work_slot_bindings"] = serde_json::json!({
            "deterministic-review": {"command": "echo", "args": []}
        });
        let parsed = InitialInput::parse(&value).unwrap();
        assert!(parsed.work_slot_bindings.is_some());
        assert_eq!(parsed.target.path, "/tmp/README.md");
        value["work_slot_bindings"] = Value::String("opaque".into());
        assert!(InitialInput::parse(&value).is_ok());
    }

    fn base() -> Value {
        let mut value: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/readme.json"
        )))
        .unwrap();
        value["target"]["path"] = Value::String("/tmp/README.md".into());
        value
    }

    #[test]
    fn rejects_invalid_modes_types_duplicates_paths_and_obligations() {
        let mut invalid = base();
        invalid["mode"] = Value::String("edit".into());
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["target"]["path"] = Value::String("README.md".into());
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["deterministic_policies"][0]["type"] = Value::String("regex".into());
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["deterministic_policies"][1]["id"] = Value::String("document-present".into());
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["semantic_policies"] = Value::Array(Vec::new());
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["deterministic_policies"][2]["aliases"] = Value::Array(Vec::new());
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["deterministic_policies"][1]["level"] = Value::from(7);
        assert!(InitialInput::parse(&invalid).is_err());
        let mut invalid = base();
        invalid["semantic_policies"][0]["description"] = Value::String(" ".into());
        assert!(InitialInput::parse(&invalid).is_err());
    }

    #[test]
    fn accepts_both_frozen_modes() {
        for mode in ["draft", "audit"] {
            let mut value = base();
            value["mode"] = Value::String(mode.into());
            assert_eq!(
                InitialInput::parse(&value).unwrap().mode,
                if mode == "draft" {
                    Mode::Draft
                } else {
                    Mode::Audit
                }
            );
        }
    }
}
