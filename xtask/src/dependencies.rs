//! Dependency policy gate (T030).
//!
//! Selected equivalent for `cargo deny check`: enforce the repository-root
//! `deny.toml` allowlists for licenses and sources, require a tracked lockfile,
//! and require advisory-policy presence. Operators may still run
//! `cargo deny check` against the same `deny.toml` for advisory-DB scanning.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use cargo_metadata::MetadataCommand;

/// Evidence owned by dependency validation, independent from scheduler records.
#[derive(Debug, Clone)]
pub struct DenyCommandEvidence {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub candidate_revision: String,
}

/// Repository-relative path to the dependency policy file.
pub const DENY_TOML: &str = "deny.toml";

/// Repository-relative path to the workspace lockfile.
pub const CARGO_LOCK: &str = "Cargo.lock";

/// crates.io registry source URL required by policy.
pub const CRATES_IO_INDEX: &str = "https://github.com/rust-lang/crates.io-index";

/// Licenses that must appear in `deny.toml` `[licenses].allow`.
pub const REQUIRED_ALLOWED_LICENSES: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "Unicode-3.0",
];

/// Pinned cargo-deny release for advisory scanning (external governance tool).
pub const CARGO_DENY_VERSION: &str = "0.20.2";

/// Fail-closed cargo-deny error carrying captured command evidence.
#[derive(Debug, Clone)]
pub struct DenyFailure {
    pub evidence: DenyCommandEvidence,
}

impl std::fmt::Display for DenyFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cargo deny check failed (exit={})",
            self.evidence.exit_code
        )
    }
}

impl std::error::Error for DenyFailure {}

/// Parsed subset of `deny.toml` consumed by the selected-equivalent gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyPolicy {
    pub allowed_licenses: BTreeSet<String>,
    pub allow_registry: BTreeSet<String>,
    pub allow_git: BTreeSet<String>,
    pub unknown_registry: LintLevel,
    pub unknown_git: LintLevel,
    pub yanked: LintLevel,
    pub has_advisories_section: bool,
    pub has_licenses_section: bool,
    pub has_sources_section: bool,
    pub wildcards: LintLevel,
}

/// Lint level used by deny.toml fields the gate understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    Deny,
    Warn,
    Allow,
}

impl LintLevel {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "deny" => Ok(Self::Deny),
            "warn" => Ok(Self::Warn),
            "allow" => Ok(Self::Allow),
            other => bail!("unsupported lint level `{other}`"),
        }
    }
}

/// Run the dependency policy gate for `root` (defaults to the current workspace).
pub fn run(root: Option<&Path>) -> Result<()> {
    let root = resolve_root(root)?;
    let policy_path = root.join(DENY_TOML);
    let policy_text = fs::read_to_string(&policy_path).with_context(|| {
        format!(
            "failed to read dependency policy at {}",
            policy_path.display()
        )
    })?;
    let policy = parse_deny_toml(&policy_text)
        .with_context(|| format!("invalid dependency policy at {}", policy_path.display()))?;
    validate_policy_shape(&policy)?;

    let lock_path = root.join(CARGO_LOCK);
    let lock_text = fs::read_to_string(&lock_path)
        .with_context(|| format!("failed to read lockfile at {}", lock_path.display()))?;
    check_lockfile_sources(&lock_text, &policy)?;

    let manifest = root.join("Cargo.toml");
    let metadata = MetadataCommand::new()
        .manifest_path(&manifest)
        .exec()
        .with_context(|| format!("failed to load cargo metadata for {}", manifest.display()))?;
    check_package_licenses(&metadata, &policy)?;

    println!("dependencies: ok ({})", root.display());
    Ok(())
}

/// Run pinned `cargo deny check` and capture exact command evidence.
pub fn run_cargo_deny_with_evidence(
    root: &Path,
    candidate_revision: &str,
) -> Result<DenyCommandEvidence> {
    ensure_cargo_deny_available()?;

    let command = format!("cargo deny check (pinned {CARGO_DENY_VERSION})");
    let mut process = Command::new("cargo");
    process.args(["deny", "check"]).current_dir(root);
    apply_quality_command_uid(&mut process)?;
    let output = process
        .output()
        .with_context(|| format!("failed to spawn cargo deny check under {}", root.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let exit_code = output.status.code().unwrap_or(-1);
    let evidence = DenyCommandEvidence {
        command,
        exit_code,
        stdout,
        stderr,
        candidate_revision: candidate_revision.to_owned(),
    };

    if output.status.success() {
        Ok(evidence)
    } else {
        bail!(DenyFailure { evidence })
    }
}

fn ensure_cargo_deny_available() -> Result<()> {
    let mut process = Command::new("cargo");
    process.args(["deny", "--version"]);
    apply_quality_command_uid(&mut process)?;
    let output = process.output().context(
        "failed to execute `cargo deny --version`; install pinned cargo-deny for publication",
    )?;
    if !output.status.success() {
        bail!(
            "cargo deny is unavailable (exit={}); install pinned cargo-deny {CARGO_DENY_VERSION} for advisory scanning",
            output.status.code().unwrap_or(-1)
        );
    }
    let version_text = String::from_utf8_lossy(&output.stdout);
    if !version_text.contains(CARGO_DENY_VERSION) {
        bail!(
            "cargo deny version mismatch: expected {CARGO_DENY_VERSION}, got `{}`",
            version_text.trim()
        );
    }
    Ok(())
}

fn apply_quality_command_uid(process: &mut Command) -> Result<()> {
    let Some(uid) = std::env::var_os("LOOP_ENGINE_QUALITY_COMMAND_UID") else {
        return Ok(());
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let uid = uid
            .to_string_lossy()
            .parse::<u32>()
            .context("invalid LOOP_ENGINE_QUALITY_COMMAND_UID")?;
        process.uid(uid).gid(uid);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = process;
        bail!("LOOP_ENGINE_QUALITY_COMMAND_UID is unsupported on this platform")
    }
}

fn resolve_root(root: Option<&Path>) -> Result<PathBuf> {
    match root {
        Some(path) => Ok(path.to_path_buf()),
        None => {
            let mut command = MetadataCommand::new();
            let metadata = command
                .no_deps()
                .exec()
                .context("failed to locate workspace root via cargo metadata")?;
            Ok(metadata.workspace_root.into_std_path_buf())
        }
    }
}

/// Parse the constrained `deny.toml` subset required by T030.
pub fn parse_deny_toml(text: &str) -> Result<DependencyPolicy> {
    let mut allowed_licenses = BTreeSet::new();
    let mut allow_registry = BTreeSet::new();
    let mut allow_git = BTreeSet::new();
    let mut unknown_registry = None;
    let mut unknown_git = None;
    let mut yanked = None;
    let mut wildcards = None;
    let mut has_advisories_section = false;
    let mut has_licenses_section = false;
    let mut has_sources_section = false;

    let mut section = String::new();
    let mut in_licenses_allow = false;
    let mut in_allow_registry = false;
    let mut in_allow_git = false;

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name) = parse_section_header(line) {
            section = name;
            in_licenses_allow = false;
            in_allow_registry = false;
            in_allow_git = false;
            match section.as_str() {
                "advisories" => has_advisories_section = true,
                "licenses" => has_licenses_section = true,
                "sources" => has_sources_section = true,
                _ => {}
            }
            continue;
        }

        if line == "allow = [" && section.as_str() == "licenses" {
            in_licenses_allow = true;
            continue;
        }
        if line == "allow-registry = [" && section == "sources" {
            in_allow_registry = true;
            continue;
        }
        if line == "allow-git = [" && section == "sources" {
            in_allow_git = true;
            continue;
        }
        if line == "]" {
            in_licenses_allow = false;
            in_allow_registry = false;
            in_allow_git = false;
            continue;
        }

        if in_licenses_allow && let Some(value) = parse_string_array_item(line) {
            allowed_licenses.insert(value);
            continue;
        }
        if in_allow_registry && let Some(value) = parse_string_array_item(line) {
            allow_registry.insert(value);
            continue;
        }
        if in_allow_git && let Some(value) = parse_string_array_item(line) {
            allow_git.insert(value);
            continue;
        }

        if section == "licenses"
            && let Some(rest) = line.strip_prefix("allow")
            && let Ok(values) = parse_inline_string_array(rest)
        {
            allowed_licenses.extend(values);
            continue;
        }
        if section == "sources" {
            if let Some(rest) = line.strip_prefix("unknown-registry") {
                unknown_registry = Some(LintLevel::parse(&parse_assignment_string(rest)?)?);
                continue;
            }
            if let Some(rest) = line.strip_prefix("unknown-git") {
                unknown_git = Some(LintLevel::parse(&parse_assignment_string(rest)?)?);
                continue;
            }
            if let Some(rest) = line.strip_prefix("allow-registry") {
                allow_registry.extend(parse_inline_string_array(rest)?);
                continue;
            }
            if let Some(rest) = line.strip_prefix("allow-git") {
                allow_git.extend(parse_inline_string_array(rest)?);
                continue;
            }
        }
        if section == "advisories"
            && let Some(rest) = line.strip_prefix("yanked")
        {
            yanked = Some(LintLevel::parse(&parse_assignment_string(rest)?)?);
            continue;
        }
        if section == "bans"
            && let Some(rest) = line.strip_prefix("wildcards")
        {
            wildcards = Some(LintLevel::parse(&parse_assignment_string(rest)?)?);
            continue;
        }
    }

    Ok(DependencyPolicy {
        allowed_licenses,
        allow_registry,
        allow_git,
        unknown_registry: unknown_registry.unwrap_or(LintLevel::Warn),
        unknown_git: unknown_git.unwrap_or(LintLevel::Warn),
        yanked: yanked.unwrap_or(LintLevel::Warn),
        has_advisories_section,
        has_licenses_section,
        has_sources_section,
        wildcards: wildcards.unwrap_or(LintLevel::Allow),
    })
}

/// Ensure deny.toml encodes the C1 governance invariants.
pub fn validate_policy_shape(policy: &DependencyPolicy) -> Result<()> {
    if !policy.has_licenses_section {
        bail!("deny.toml must define a [licenses] section");
    }
    if !policy.has_sources_section {
        bail!("deny.toml must define a [sources] section");
    }
    if !policy.has_advisories_section {
        bail!("deny.toml must define an [advisories] section");
    }
    if policy.allowed_licenses.is_empty() {
        bail!("deny.toml [licenses].allow must be non-empty");
    }
    for required in REQUIRED_ALLOWED_LICENSES {
        if !policy.allowed_licenses.contains(*required) {
            bail!("deny.toml [licenses].allow must include `{required}`");
        }
    }
    if policy.unknown_registry != LintLevel::Deny {
        bail!("deny.toml [sources].unknown-registry must be deny");
    }
    if policy.unknown_git != LintLevel::Deny {
        bail!("deny.toml [sources].unknown-git must be deny");
    }
    if !policy.allow_registry.contains(CRATES_IO_INDEX) {
        bail!("deny.toml [sources].allow-registry must include crates.io index");
    }
    if !policy.allow_git.is_empty() {
        bail!("deny.toml [sources].allow-git must be empty for C1");
    }
    if policy.yanked != LintLevel::Deny {
        bail!("deny.toml [advisories].yanked must be deny");
    }
    if policy.wildcards != LintLevel::Deny {
        bail!("deny.toml [bans].wildcards must be deny");
    }
    Ok(())
}

/// Reject non-crates.io / git lockfile sources according to policy.
pub fn check_lockfile_sources(lock_text: &str, policy: &DependencyPolicy) -> Result<()> {
    let mut package_name = None;
    let mut package_version = None;
    let mut violations = Vec::new();

    for raw_line in lock_text.lines() {
        let line = raw_line.trim();
        if line == "[[package]]" {
            package_name = None;
            package_version = None;
            continue;
        }
        if let Some(name) = line.strip_prefix("name = \"") {
            package_name = Some(name.trim_end_matches('"').to_string());
            continue;
        }
        if let Some(version) = line.strip_prefix("version = \"") {
            package_version = Some(version.trim_end_matches('"').to_string());
            continue;
        }
        if let Some(source) = line.strip_prefix("source = \"") {
            let source = source.trim_end_matches('"');
            let label = match (&package_name, &package_version) {
                (Some(name), Some(version)) => format!("{name}@{version}"),
                (Some(name), None) => name.clone(),
                _ => "<unknown>".to_string(),
            };
            if let Some(registry) = source.strip_prefix("registry+") {
                if !policy.allow_registry.contains(registry)
                    && policy.unknown_registry == LintLevel::Deny
                {
                    violations.push(format!(
                        "lockfile package {label} uses disallowed registry `{registry}`"
                    ));
                }
            } else if source.starts_with("git+") {
                if policy.unknown_git == LintLevel::Deny {
                    violations.push(format!(
                        "lockfile package {label} uses disallowed git source `{source}`"
                    ));
                }
            } else if policy.unknown_registry == LintLevel::Deny {
                violations.push(format!(
                    "lockfile package {label} uses disallowed source `{source}`"
                ));
            }
        }
    }

    if !violations.is_empty() {
        bail!("{}", violations.join("\n"));
    }
    Ok(())
}

/// Reject package license expressions that cannot satisfy the allowlist.
pub fn check_package_licenses(
    metadata: &cargo_metadata::Metadata,
    policy: &DependencyPolicy,
) -> Result<()> {
    let mut violations = Vec::new();
    for package in &metadata.packages {
        let Some(raw) = package.license.as_deref() else {
            violations.push(format!(
                "package {}@{} has no license metadata",
                package.name, package.version
            ));
            continue;
        };
        let normalized = normalize_license_expression(raw);
        if !license_expression_allowed(&normalized, &policy.allowed_licenses) {
            violations.push(format!(
                "package {}@{} license `{raw}` is not allowed by deny.toml",
                package.name, package.version
            ));
        }
    }
    if !violations.is_empty() {
        bail!("{}", violations.join("\n"));
    }
    Ok(())
}

/// Normalize common Cargo license spelling quirks into SPDX-ish OR form.
pub fn normalize_license_expression(raw: &str) -> String {
    raw.replace('/', " OR ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Evaluate a normalized SPDX-ish expression against an allowlist.
///
/// Supports `OR`, `AND`, `WITH`, and parentheses. An `OR` expression passes when
/// any alternative passes; an `AND` expression requires every conjunct.
pub fn license_expression_allowed(expr: &str, allowed: &BTreeSet<String>) -> bool {
    match parse_or(expr.trim()) {
        Some(value) => eval_or(&value, allowed),
        None => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expr {
    License(String),
    Or(Vec<Expr>),
    And(Vec<Expr>),
}

fn eval_or(expr: &Expr, allowed: &BTreeSet<String>) -> bool {
    match expr {
        Expr::License(id) => allowed.contains(id),
        Expr::Or(items) => items.iter().any(|item| eval_or(item, allowed)),
        Expr::And(items) => items.iter().all(|item| eval_or(item, allowed)),
    }
}

fn parse_or(input: &str) -> Option<Expr> {
    let parts = split_top_level(input, "OR")?;
    if parts.len() == 1 {
        return parse_and(parts[0]);
    }
    let mut items = Vec::new();
    for part in parts {
        items.push(parse_and(part)?);
    }
    Some(Expr::Or(items))
}

fn parse_and(input: &str) -> Option<Expr> {
    let parts = split_top_level(input, "AND")?;
    if parts.len() == 1 {
        return parse_primary(parts[0]);
    }
    let mut items = Vec::new();
    for part in parts {
        items.push(parse_primary(part)?);
    }
    Some(Expr::And(items))
}

fn parse_primary(input: &str) -> Option<Expr> {
    let trimmed = input.trim();
    if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')'))
        && balanced(inner)
    {
        return parse_or(inner);
    }
    // Keep WITH exceptions attached to the license id.
    Some(Expr::License(trimmed.to_string()))
}

fn split_top_level<'a>(input: &'a str, sep: &str) -> Option<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = input.as_bytes();
    let sep_bytes = sep.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            _ if depth == 0
                && i + sep_bytes.len() <= bytes.len()
                && &bytes[i..i + sep_bytes.len()] == sep_bytes
                && is_sep_boundary_before(bytes, i)
                && is_sep_boundary_after(bytes, i + sep_bytes.len()) =>
            {
                parts.push(input[start..i].trim());
                i += sep_bytes.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(input[start..].trim());
    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    Some(parts)
}

fn is_sep_boundary_before(bytes: &[u8], index: usize) -> bool {
    index == 0 || !bytes[index - 1].is_ascii_alphanumeric()
}

fn is_sep_boundary_after(bytes: &[u8], index: usize) -> bool {
    index == bytes.len() || !bytes[index].is_ascii_alphanumeric()
}

fn balanced(input: &str) -> bool {
    let mut depth = 0i32;
    for ch in input.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}

fn parse_section_header(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.is_empty() || inner.contains('[') || inner.contains(']') {
        return None;
    }
    // Use leaf section name: licenses.private -> private under parent tracking
    // is unnecessary for this subset parser; keep full dotted name where useful.
    Some(inner.to_string())
}

fn parse_string_array_item(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_end_matches(',');
    let trimmed = trimmed.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return Some(trimmed[1..trimmed.len() - 1].to_string());
    }
    None
}

fn parse_assignment_string(rest: &str) -> Result<String> {
    let rest = rest.trim();
    let rest = rest
        .strip_prefix('=')
        .map(str::trim)
        .context("expected `=` in deny.toml assignment")?;
    if rest.len() >= 2 && rest.starts_with('"') && rest.ends_with('"') {
        return Ok(rest[1..rest.len() - 1].to_string());
    }
    // bare ident: deny/warn/allow
    if rest
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Ok(rest.to_string());
    }
    bail!("expected string or ident value, found `{rest}`");
}

fn parse_inline_string_array(rest: &str) -> Result<Vec<String>> {
    let rest = rest.trim();
    let rest = rest
        .strip_prefix('=')
        .map(str::trim)
        .context("expected `=` before array")?;
    let rest = rest
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .context("expected inline string array")?;
    let mut values = Vec::new();
    for piece in rest.split(',') {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        if piece.len() >= 2 && piece.starts_with('"') && piece.ends_with('"') {
            values.push(piece[1..piece.len() - 1].to_string());
        } else {
            bail!("expected quoted string in array, found `{piece}`");
        }
    }
    Ok(values)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn license_or_expression_accepts_any_allowed_alternative() {
        let allowed = ["MIT", "Apache-2.0"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(license_expression_allowed(
            "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
            &allowed
        ));
    }

    #[test]
    fn license_and_expression_requires_every_conjunct() {
        let allowed = ["MIT", "Apache-2.0", "Unicode-3.0"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(license_expression_allowed(
            "(MIT OR Apache-2.0) AND Unicode-3.0",
            &allowed
        ));
        let missing_unicode = ["MIT", "Apache-2.0"]
            .into_iter()
            .map(str::to_string)
            .collect();
        assert!(!license_expression_allowed(
            "(MIT OR Apache-2.0) AND Unicode-3.0",
            &missing_unicode
        ));
    }

    #[test]
    fn slash_license_normalizes_to_or() {
        assert_eq!(
            normalize_license_expression("MIT/Apache-2.0"),
            "MIT OR Apache-2.0"
        );
        assert_eq!(
            normalize_license_expression("Unlicense/MIT"),
            "Unlicense OR MIT"
        );
    }
}
