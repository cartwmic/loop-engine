//! Argv-controlled fixture modes for drift, compatibility, guidance, and graph variants.

use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompatMode {
    #[default]
    Compatible,
    Incompatible,
    EvaluationError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuidanceMode {
    #[default]
    Default,
    RecommendEvidence,
    Incompatible,
    EvaluationError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DescribeGraphVariant {
    #[default]
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    pub provider_version: Option<String>,
    pub compat_mode: CompatMode,
    pub guidance_mode: GuidanceMode,
    pub describe_graph: DescribeGraphVariant,
    /// When set, evaluate_gates returns evaluation_error (deterministic test hook).
    pub gate_evaluation_error: bool,
    /// When set, evaluate_gates returns incompatible (deterministic test hook).
    pub gate_incompatible: bool,
    /// When set, emit intentionally malformed provider evidence (deterministic test hook).
    pub malformed_evidence: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            provider_version: Some("reference-provider/1.0.0".to_string()),
            compat_mode: CompatMode::Compatible,
            guidance_mode: GuidanceMode::Default,
            describe_graph: DescribeGraphVariant::V1,
            gate_evaluation_error: false,
            gate_incompatible: false,
            malformed_evidence: false,
        }
    }
}

impl ProviderConfig {
    pub fn from_process_argv() -> Self {
        from_argv(env::args().skip(1).collect())
    }

    pub fn merge_registration_argv(&mut self, argv: &[String]) {
        apply_argv(self, argv.iter().map(String::as_str));
    }
}

fn from_argv(argv: Vec<String>) -> ProviderConfig {
    let mut config = ProviderConfig::default();
    apply_argv(&mut config, argv.iter().map(String::as_str));
    config
}

fn apply_argv<'a>(config: &mut ProviderConfig, argv: impl IntoIterator<Item = &'a str>) {
    for arg in argv {
        if let Some(value) = arg.strip_prefix("--provider-version=") {
            config.provider_version = Some(value.to_string());
        } else if let Some(value) = arg.strip_prefix("--compat=") {
            config.compat_mode = match value {
                "incompatible" => CompatMode::Incompatible,
                "evaluation_error" => CompatMode::EvaluationError,
                _ => CompatMode::Compatible,
            };
        } else if let Some(value) = arg.strip_prefix("--guidance=") {
            config.guidance_mode = match value {
                "recommend" => GuidanceMode::RecommendEvidence,
                "incompatible" => GuidanceMode::Incompatible,
                "evaluation_error" => GuidanceMode::EvaluationError,
                _ => GuidanceMode::Default,
            };
        } else if let Some(value) = arg.strip_prefix("--describe-graph=") {
            config.describe_graph = match value {
                "v2" => DescribeGraphVariant::V2,
                _ => DescribeGraphVariant::V1,
            };
        } else if arg == "--gate-evaluation-error" {
            config.gate_evaluation_error = true;
        } else if arg == "--gate-incompatible" {
            config.gate_incompatible = true;
        } else if arg == "--malformed-evidence" {
            config.malformed_evidence = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_argv_modes() {
        let config = from_argv(vec![
            "--provider-version=2.0.0-test".to_string(),
            "--compat=incompatible".to_string(),
            "--guidance=recommend".to_string(),
            "--describe-graph=v2".to_string(),
            "--gate-evaluation-error".to_string(),
        ]);
        assert_eq!(config.provider_version.as_deref(), Some("2.0.0-test"));
        assert_eq!(config.compat_mode, CompatMode::Incompatible);
        assert_eq!(config.guidance_mode, GuidanceMode::RecommendEvidence);
        assert_eq!(config.describe_graph, DescribeGraphVariant::V2);
        assert!(config.gate_evaluation_error);
    }
}
