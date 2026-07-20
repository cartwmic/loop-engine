use loop_engine_core::model::bounded::PROVIDER_TIMEOUT_SECONDS_DEFAULT;
use loop_engine_core::model::ids::RegistrationId;

use super::dto::{ConfigurationDto, OutputFormat};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliDefaults {
    pub format: Option<OutputFormat>,
    pub provider: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDefaults {
    pub format: OutputFormat,
    pub provider: Option<String>,
    pub timeout_seconds: u64,
}

pub fn resolve_defaults(
    cli: &CliDefaults,
    project: Option<&ConfigurationDto>,
    global: Option<&ConfigurationDto>,
) -> ResolvedDefaults {
    let project = project.map(|value| &value.defaults);
    let global = global.map(|value| &value.defaults);
    ResolvedDefaults {
        format: first(
            cli.format,
            project.and_then(|value| value.format),
            global.and_then(|value| value.format),
        )
        .unwrap_or(OutputFormat::Human),
        provider: first_ref(
            cli.provider.as_ref(),
            project.and_then(|value| value.provider.as_ref()),
            global.and_then(|value| value.provider.as_ref()),
        )
        .cloned(),
        timeout_seconds: first(
            cli.timeout_seconds,
            project.and_then(|value| value.timeout_seconds),
            global.and_then(|value| value.timeout_seconds),
        )
        .unwrap_or(PROVIDER_TIMEOUT_SECONDS_DEFAULT),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetProviderRequirement {
    Required,
    Optional,
}

pub fn provider_for_new_target<'a>(
    explicit: Option<&'a str>,
    defaults: &'a ResolvedDefaults,
    requirement: TargetProviderRequirement,
) -> Option<&'a str> {
    explicit.or(match requirement {
        TargetProviderRequirement::Required => None,
        TargetProviderRequirement::Optional => defaults.provider.as_deref(),
    })
}

pub fn provider_for_existing_run(stored: &RegistrationId) -> &RegistrationId {
    stored
}

fn first<T: Copy>(cli: Option<T>, project: Option<T>, global: Option<T>) -> Option<T> {
    cli.or(project).or(global)
}

fn first_ref<'a, T>(
    cli: Option<&'a T>,
    project: Option<&'a T>,
    global: Option<&'a T>,
) -> Option<&'a T> {
    cli.or(project).or(global)
}
