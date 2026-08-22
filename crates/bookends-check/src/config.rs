use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoConfig {
    pub prd: String,
    pub e2e_journey: ClassConfig,
    pub contract: Option<ClassConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClassConfig {
    pub pathspecs: Vec<String>,
    pub required_ci_jobs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    prd: String,
    classes: RawClasses,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClasses {
    e2e_journey: RawClass,
    #[serde(default)]
    contract: Option<RawClass>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClass {
    pathspecs: Vec<String>,
    required_ci_jobs: Vec<String>,
}

pub(crate) fn parse_repo_config(text: &str) -> Result<RepoConfig, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|err| format!("bookends.toml: {err}"))?;
    validate_prd_path(&raw.prd)?;
    let e2e_journey = validate_class("classes.e2e_journey", raw.classes.e2e_journey)?;
    let contract = match raw.classes.contract {
        Some(class) => Some(validate_class("classes.contract", class)?),
        None => None,
    };
    Ok(RepoConfig {
        prd: raw.prd,
        e2e_journey,
        contract,
    })
}

fn validate_prd_path(prd: &str) -> Result<(), String> {
    let normalized = prd.replace('\\', "/");
    let windows_drive = normalized.len() >= 2
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':';
    if prd.is_empty() {
        return Err("bookends.toml: prd must be a non-empty repo-relative path".into());
    }
    if normalized.starts_with('/') || windows_drive || prd.contains("://") {
        return Err(
            "bookends.toml: prd must be a repo-relative markdown path, not an absolute path or URL"
                .into(),
        );
    }
    if normalized.split('/').any(|part| part == "..") {
        return Err("bookends.toml: prd must not contain parent-directory segments".into());
    }
    if !(normalized.ends_with(".md") || normalized.ends_with(".markdown")) {
        return Err("bookends.toml: prd must name a markdown file".into());
    }
    Ok(())
}

fn validate_class(name: &str, raw: RawClass) -> Result<ClassConfig, String> {
    if raw.pathspecs.is_empty() {
        return Err(format!("{name}: pathspecs must be a nonempty array"));
    }
    if raw.pathspecs.iter().any(|p| p.is_empty()) {
        return Err(format!(
            "{name}: pathspecs entries must be nonempty strings"
        ));
    }
    if raw.required_ci_jobs.is_empty() {
        return Err(format!("{name}: required_ci_jobs must be a nonempty array"));
    }
    if raw.required_ci_jobs.iter().any(|j| j.is_empty()) {
        return Err(format!(
            "{name}: required_ci_jobs entries must be nonempty strings"
        ));
    }
    Ok(ClassConfig {
        pathspecs: raw.pathspecs,
        required_ci_jobs: raw.required_ci_jobs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_required_keys() {
        let cfg = parse_repo_config(
            r#"
prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.prd, "docs/PRD.md");
        assert!(cfg.contract.is_none());
    }

    #[test]
    fn unknown_class_table_is_invalid() {
        let err = parse_repo_config(
            r#"
prd = "docs/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]

[classes.go]
pathspecs = ["go/**"]
required_ci_jobs = ["journey"]
"#,
        )
        .unwrap_err();
        assert!(err.contains("bookends.toml"), "{err}");
    }

    #[test]
    fn absolute_prd_is_invalid() {
        let err = parse_repo_config(
            r#"
prd = "/tmp/PRD.md"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]
"#,
        )
        .unwrap_err();
        assert!(err.contains("repo-relative"), "{err}");
    }

    #[test]
    fn non_markdown_prd_is_invalid() {
        let err = parse_repo_config(
            r#"
prd = "docs/PRD.txt"

[classes.e2e_journey]
pathspecs = ["tests/**"]
required_ci_jobs = ["journey"]
"#,
        )
        .unwrap_err();
        assert!(err.contains("markdown"), "{err}");
    }
}
