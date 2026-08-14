//! Initial-input parsing and obligation-independent configuration validation.
//!
//! This module owns the provider's configuration namespace and subject
//! metadata invariants.  It does not read `artifact_root` or know anything
//! about transition routing; callers query the semantically keyed result.

#![allow(dead_code)]

use crate::schema::{validate_schema, MetaValidationReport, ValidatedSchema};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Gate identifiers accepted in `review_policies`.
///
/// This is the policy namespace from technical-design §8.  Transition tuple
/// mapping remains owned by `gates.rs`.
pub(crate) const GATE_IDS: &[&str] = &["verify", "synthesize"];

/// Artifact subject names accepted in `artifact_schemas` and revision links.
pub(crate) const SUBJECT_NAMES: &[&str] = &[
    "brief.json",
    "sources.json",
    "verification.json",
    "report.json",
];

const TOP_LEVEL_KEYS: &[&str] = &[
    "config_version",
    "artifact_root",
    "review_policies",
    "artifact_schemas",
    "revision_links",
    "extra",
];

const REVISION_LINK_KEYS: &[&str] = &["from", "field", "to"];
const SHIPPED_CONFIG_NAMES: &str = "standard";
const AUTHOR_KINDS: &[&str] = &["human", "agent", "script"];

/// One configured policy axis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyAxis {
    id: String,
    description: String,
    required_authors: u64,
}

impl PolicyAxis {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn description(&self) -> &str {
        &self.description
    }

    pub(crate) fn required_authors(&self) -> u64 {
        self.required_authors
    }
}

/// One declarative cross-artifact revision link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RevisionLink {
    from: String,
    field: String,
    to: String,
}

impl RevisionLink {
    pub(crate) fn from(&self) -> &str {
        &self.from
    }

    pub(crate) fn field(&self) -> &str {
        &self.field
    }

    pub(crate) fn to(&self) -> &str {
        &self.to
    }
}

/// Configuration accepted after all config-level checks have passed.
///
/// Every collection is semantically keyed for downstream evaluation:
/// schemas by subject, links by their `from` subject, and policy axes by gate
/// then axis id.  `artifact_root` is retained as raw JSON and intentionally
/// remains unexamined here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedConfig {
    config_version: String,
    artifact_root: Option<Value>,
    extra: Option<Value>,
    schemas_by_subject: BTreeMap<String, ValidatedSchema>,
    links_by_from: BTreeMap<String, Vec<RevisionLink>>,
    axes_by_gate: BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    axis_namespace: BTreeMap<String, BTreeSet<String>>,
}

impl ValidatedConfig {
    pub(crate) fn config_version(&self) -> &str {
        &self.config_version
    }

    /// Return raw caller input.  No path, type, existence, or containment
    /// checks happen in this module.
    pub(crate) fn artifact_root(&self) -> Option<&Value> {
        self.artifact_root.as_ref()
    }

    pub(crate) fn extra(&self) -> Option<&Value> {
        self.extra.as_ref()
    }

    /// Schemas keyed by fixed subject name.
    pub(crate) fn schemas_by_subject(&self) -> &BTreeMap<String, ValidatedSchema> {
        &self.schemas_by_subject
    }

    pub(crate) fn schema(&self, subject: &str) -> Option<&ValidatedSchema> {
        self.schemas_by_subject.get(subject)
    }

    /// Revision links keyed by `from` subject name.
    pub(crate) fn links_by_from(&self) -> &BTreeMap<String, Vec<RevisionLink>> {
        &self.links_by_from
    }

    pub(crate) fn links_from(&self, subject: &str) -> &[RevisionLink] {
        self.links_by_from
            .get(subject)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Policy axes keyed by gate and then policy id.
    pub(crate) fn axes_by_gate(&self) -> &BTreeMap<String, BTreeMap<String, PolicyAxis>> {
        &self.axes_by_gate
    }

    pub(crate) fn axes_for_gate(&self, gate: &str) -> Option<&BTreeMap<String, PolicyAxis>> {
        self.axes_by_gate.get(gate)
    }

    pub(crate) fn axis(&self, gate: &str, axis_id: &str) -> Option<&PolicyAxis> {
        self.axes_by_gate.get(gate)?.get(axis_id)
    }

    /// Full configured gate/axis namespace, including records for other
    /// configured gates that evidence attribution must silently isolate.
    pub(crate) fn axis_namespace(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.axis_namespace
    }
}

/// A specific malformed-config classification.
///
/// Keeping classes as enum variants is deliberate: evaluation can map every
/// variant to its operational error path without confusing malformed config
/// with schema or evidence denial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConfigViolation {
    TopLevelNotObject,
    UnknownTopLevelKey {
        key: String,
    },
    MissingConfigVersion,
    ConfigVersionNotString,
    ConfigVersionEmpty,
    MissingReviewPolicies,
    ReviewPoliciesNotObject,
    UnknownGateName {
        gate: String,
    },
    PolicyAxesNotArray {
        gate: String,
    },
    PolicyEntryNotObject {
        gate: String,
        index: usize,
    },
    PolicyIdMissing {
        gate: String,
        index: usize,
    },
    PolicyIdNotString {
        gate: String,
        index: usize,
    },
    PolicyIdEmpty {
        gate: String,
        index: usize,
    },
    PolicyDescriptionMissing {
        gate: String,
        index: usize,
    },
    PolicyDescriptionNotString {
        gate: String,
        index: usize,
    },
    RequiredAuthorsInvalid {
        gate: String,
        index: usize,
    },
    DuplicateAxisId {
        gate: String,
        id: String,
    },
    ArtifactSchemasNotObject,
    UnknownArtifactName {
        subject: String,
    },
    SchemaInvalid {
        subject: String,
        report: MetaValidationReport,
    },
    RevisionLinksNotArray,
    MalformedRevisionLink {
        index: usize,
        kind: RevisionLinkViolation,
    },
    AxesRequireSchema {
        gate: String,
        subject: String,
    },
    SchemaMetadataInvalid {
        subject: String,
        issues: Vec<String>,
    },
    LinkRequiresSchemas {
        index: usize,
        from: String,
        to: String,
        issues: Vec<LinkSchemaViolation>,
    },
}

/// Shape/name failure inside one `revision_links` entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RevisionLinkViolation {
    EntryNotObject,
    MissingField { field: String },
    WrongType { field: String },
    ExtraField { field: String },
    UnknownSubject { field: String, value: String },
    EmptyField { field: String },
}

/// Subject-schema invariant failure for one revision link.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LinkSchemaViolation {
    MissingFromSchema,
    MissingToSchema,
    ToSchemaMissingRevision,
}

/// Complete config-validation error report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConfigValidationError {
    violations: Vec<ConfigViolation>,
}

impl ConfigValidationError {
    fn new(mut violations: Vec<ConfigViolation>) -> Self {
        violations.sort_by_key(config_violation_sort_key);
        Self { violations }
    }

    pub(crate) fn violations(&self) -> &[ConfigViolation] {
        &self.violations
    }

    pub(crate) fn into_violations(self) -> Vec<ConfigViolation> {
        self.violations
    }

    pub(crate) fn is_class(&self, class: &str) -> bool {
        self.violations
            .iter()
            .any(|violation| violation.class() == class)
    }
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, violation) in self.violations.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            violation.fmt(formatter)?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigValidationError {}

impl ConfigViolation {
    /// Stable high-level class name for diagnostics and table-driven tests.
    pub(crate) fn class(&self) -> &'static str {
        match self {
            Self::TopLevelNotObject => "top-level-shape",
            Self::UnknownTopLevelKey { .. } => "unknown-top-level-key",
            Self::MissingConfigVersion => "config-version-missing",
            Self::ConfigVersionNotString => "config-version-not-string",
            Self::ConfigVersionEmpty => "config-version-empty",
            Self::MissingReviewPolicies => "review-policies-missing",
            Self::ReviewPoliciesNotObject => "review-policies-shape",
            Self::UnknownGateName { .. } => "unknown-gate-name",
            Self::PolicyAxesNotArray { .. } => "policy-axes-shape",
            Self::PolicyEntryNotObject { .. } => "policy-entry-shape",
            Self::PolicyIdMissing { .. } => "policy-id-missing",
            Self::PolicyIdNotString { .. } => "policy-id-not-string",
            Self::PolicyIdEmpty { .. } => "policy-id-empty",
            Self::PolicyDescriptionMissing { .. } => "policy-description-missing",
            Self::PolicyDescriptionNotString { .. } => "policy-description-not-string",
            Self::RequiredAuthorsInvalid { .. } => "bad-required-authors",
            Self::DuplicateAxisId { .. } => "duplicate-axis-id",
            Self::ArtifactSchemasNotObject => "artifact-schemas-shape",
            Self::UnknownArtifactName { .. } => "unknown-artifact-name",
            Self::SchemaInvalid { .. } => "schema-invalid",
            Self::RevisionLinksNotArray => "revision-links-shape",
            Self::MalformedRevisionLink { .. } => "malformed-revision-link",
            Self::AxesRequireSchema { .. } => "axes-require-schema",
            Self::SchemaMetadataInvalid { .. } => "schema-metadata-invalid",
            Self::LinkRequiresSchemas { .. } => "links-require-schemas",
        }
    }
}

impl fmt::Display for ConfigViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TopLevelNotObject => {
                write!(
                    formatter,
                    "{}: initial input must be a JSON object",
                    self.class()
                )
            }
            Self::UnknownTopLevelKey { key } => {
                write!(formatter, "{}: unknown top-level key `{key}`", self.class())
            }
            Self::MissingConfigVersion => {
                write!(formatter, "{}: missing `config_version`", self.class())
            }
            Self::ConfigVersionNotString => write!(
                formatter,
                "{}: `config_version` must be a string",
                self.class()
            ),
            Self::ConfigVersionEmpty => write!(
                formatter,
                "{}: `config_version` must not be empty",
                self.class()
            ),
            Self::MissingReviewPolicies => write!(
                formatter,
                "{}: missing `review_policies`; use shipped configs: {SHIPPED_CONFIG_NAMES}",
                self.class()
            ),
            Self::ReviewPoliciesNotObject => write!(
                formatter,
                "{}: `review_policies` must be an object",
                self.class()
            ),
            Self::UnknownGateName { gate } => {
                write!(formatter, "{}: unknown policy gate `{gate}`", self.class())
            }
            Self::PolicyAxesNotArray { gate } => write!(
                formatter,
                "{}: policy gate `{gate}` must contain an array",
                self.class()
            ),
            Self::PolicyEntryNotObject { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` must be an object",
                self.class()
            ),
            Self::PolicyIdMissing { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` missing `id`",
                self.class()
            ),
            Self::PolicyIdNotString { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` `id` must be a string",
                self.class()
            ),
            Self::PolicyIdEmpty { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` `id` must not be empty",
                self.class()
            ),
            Self::PolicyDescriptionMissing { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` missing `description`",
                self.class()
            ),
            Self::PolicyDescriptionNotString { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` `description` must be a string",
                self.class()
            ),
            Self::RequiredAuthorsInvalid { gate, index } => write!(
                formatter,
                "{}: policy `{gate}[{index}]` `required_authors` must be an integer >= 1",
                self.class()
            ),
            Self::DuplicateAxisId { gate, id } => write!(
                formatter,
                "{}: duplicate policy id `{id}` in gate `{gate}`",
                self.class()
            ),
            Self::ArtifactSchemasNotObject => write!(
                formatter,
                "{}: `artifact_schemas` must be an object",
                self.class()
            ),
            Self::UnknownArtifactName { subject } => write!(
                formatter,
                "{}: unknown artifact subject `{subject}`",
                self.class()
            ),
            Self::SchemaInvalid { subject, report } => write!(
                formatter,
                "{}: schema `{subject}` is invalid: {}",
                self.class(),
                report
                    .violations()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::RevisionLinksNotArray => write!(
                formatter,
                "{}: `revision_links` must be an array",
                self.class()
            ),
            Self::MalformedRevisionLink { index, kind } => write!(
                formatter,
                "{}: revision_links[{index}] {kind}",
                self.class()
            ),
            Self::AxesRequireSchema { gate, subject } => write!(
                formatter,
                "{}: gate `{gate}` has policy axes but no schema for `{subject}`",
                self.class()
            ),
            Self::SchemaMetadataInvalid { subject, issues } => write!(
                formatter,
                "{}: schema `{subject}` lacks required subject metadata: {}",
                self.class(),
                issues.join(", ")
            ),
            Self::LinkRequiresSchemas {
                index,
                from,
                to,
                issues,
            } => write!(
                formatter,
                "{}: revision_links[{index}] `{from}` -> `{to}`: {}",
                self.class(),
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl fmt::Display for RevisionLinkViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntryNotObject => formatter.write_str("entry must be an object"),
            Self::MissingField { field } => write!(formatter, "missing `{field}`"),
            Self::WrongType { field } => write!(formatter, "`{field}` must be a string"),
            Self::ExtraField { field } => write!(formatter, "unknown field `{field}`"),
            Self::UnknownSubject { field, value } => {
                write!(formatter, "`{field}` has unknown subject `{value}`")
            }
            Self::EmptyField { field } => write!(formatter, "`{field}` must not be empty"),
        }
    }
}

impl fmt::Display for LinkSchemaViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFromSchema => formatter.write_str("from subject has no configured schema"),
            Self::MissingToSchema => formatter.write_str("to subject has no configured schema"),
            Self::ToSchemaMissingRevision => {
                formatter.write_str("to subject schema must require string `revision`")
            }
        }
    }
}

/// Parse and fully meta-validate one initial-input value.
///
/// `artifact_root` is copied without inspecting its type or contents.  The
/// evaluation algorithm validates it only when a selected obligation needs a
/// schema/link read.
pub(crate) fn parse_initial_input(
    initial_input: &Value,
) -> Result<ValidatedConfig, ConfigValidationError> {
    let Some(root) = initial_input.as_object() else {
        return Err(ConfigValidationError::new(vec![
            ConfigViolation::TopLevelNotObject,
        ]));
    };

    let mut violations = Vec::new();
    for key in root.keys() {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            violations.push(ConfigViolation::UnknownTopLevelKey { key: key.clone() });
        }
    }

    let config_version = match root.get("config_version") {
        None => {
            violations.push(ConfigViolation::MissingConfigVersion);
            None
        }
        Some(Value::String(value)) if value.is_empty() => {
            violations.push(ConfigViolation::ConfigVersionEmpty);
            None
        }
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            violations.push(ConfigViolation::ConfigVersionNotString);
            None
        }
    };

    let mut axes_by_gate = BTreeMap::new();
    if !root.contains_key("review_policies") {
        violations.push(ConfigViolation::MissingReviewPolicies);
    } else if let Some(value) = root.get("review_policies") {
        parse_review_policies(value, &mut axes_by_gate, &mut violations);
    }

    let mut raw_schemas = BTreeMap::new();
    let mut schemas_by_subject = BTreeMap::new();
    if let Some(value) = root.get("artifact_schemas") {
        parse_schemas(
            value,
            &mut raw_schemas,
            &mut schemas_by_subject,
            &mut violations,
        );
    }

    let mut parsed_links: Vec<(usize, RevisionLink)> = Vec::new();
    if let Some(value) = root.get("revision_links") {
        parse_revision_links(value, &mut parsed_links, &mut violations);
    }
    let mut links_by_from: BTreeMap<String, Vec<RevisionLink>> = BTreeMap::new();
    for (_, link) in &parsed_links {
        links_by_from
            .entry(link.from.clone())
            .or_default()
            .push(link.clone());
    }

    validate_subject_metadata(&axes_by_gate, &raw_schemas, &parsed_links, &mut violations);

    if !violations.is_empty() {
        return Err(ConfigValidationError::new(violations));
    }

    let config_version = config_version.expect("missing config_version is a violation");
    let axis_namespace = axes_by_gate
        .iter()
        .map(|(gate, axes)| (gate.clone(), axes.keys().cloned().collect::<BTreeSet<_>>()))
        .collect();

    Ok(ValidatedConfig {
        config_version,
        artifact_root: root.get("artifact_root").cloned(),
        extra: root.get("extra").cloned(),
        schemas_by_subject,
        links_by_from,
        axes_by_gate,
        axis_namespace,
    })
}

/// Alias with config-oriented naming for downstream callers.
pub(crate) fn validate_config(
    initial_input: &Value,
) -> Result<ValidatedConfig, ConfigValidationError> {
    parse_initial_input(initial_input)
}

fn parse_review_policies(
    value: &Value,
    axes_by_gate: &mut BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    violations: &mut Vec<ConfigViolation>,
) {
    let Some(gates) = value.as_object() else {
        violations.push(ConfigViolation::ReviewPoliciesNotObject);
        return;
    };

    for (gate, entries) in gates {
        if !is_known_gate(gate) {
            violations.push(ConfigViolation::UnknownGateName { gate: gate.clone() });
            continue;
        }

        let Some(entries) = entries.as_array() else {
            violations.push(ConfigViolation::PolicyAxesNotArray { gate: gate.clone() });
            continue;
        };

        let mut axes = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let Some(entry) = entry.as_object() else {
                violations.push(ConfigViolation::PolicyEntryNotObject {
                    gate: gate.clone(),
                    index,
                });
                continue;
            };

            let id = match entry.get("id") {
                None => {
                    violations.push(ConfigViolation::PolicyIdMissing {
                        gate: gate.clone(),
                        index,
                    });
                    None
                }
                Some(Value::String(value)) if value.is_empty() => {
                    violations.push(ConfigViolation::PolicyIdEmpty {
                        gate: gate.clone(),
                        index,
                    });
                    None
                }
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => {
                    violations.push(ConfigViolation::PolicyIdNotString {
                        gate: gate.clone(),
                        index,
                    });
                    None
                }
            };

            let description = match entry.get("description") {
                None => {
                    violations.push(ConfigViolation::PolicyDescriptionMissing {
                        gate: gate.clone(),
                        index,
                    });
                    None
                }
                Some(Value::String(value)) => Some(value.clone()),
                Some(_) => {
                    violations.push(ConfigViolation::PolicyDescriptionNotString {
                        gate: gate.clone(),
                        index,
                    });
                    None
                }
            };

            let required_authors = match entry.get("required_authors") {
                None => Some(1),
                Some(value) => match value.as_u64() {
                    Some(value) if value >= 1 => Some(value),
                    _ => {
                        violations.push(ConfigViolation::RequiredAuthorsInvalid {
                            gate: gate.clone(),
                            index,
                        });
                        None
                    }
                },
            };

            let (Some(id), Some(description), Some(required_authors)) =
                (id, description, required_authors)
            else {
                continue;
            };

            if axes.contains_key(&id) {
                violations.push(ConfigViolation::DuplicateAxisId {
                    gate: gate.clone(),
                    id,
                });
                continue;
            }

            axes.insert(
                id.clone(),
                PolicyAxis {
                    id,
                    description,
                    required_authors,
                },
            );
        }

        axes_by_gate.insert(gate.clone(), axes);
    }
}

fn parse_schemas(
    value: &Value,
    raw_schemas: &mut BTreeMap<String, Value>,
    schemas_by_subject: &mut BTreeMap<String, ValidatedSchema>,
    violations: &mut Vec<ConfigViolation>,
) {
    let Some(schemas) = value.as_object() else {
        violations.push(ConfigViolation::ArtifactSchemasNotObject);
        return;
    };

    for (subject, schema) in schemas {
        if !is_known_subject(subject) {
            violations.push(ConfigViolation::UnknownArtifactName {
                subject: subject.clone(),
            });
            continue;
        }

        raw_schemas.insert(subject.clone(), schema.clone());
        match validate_schema(schema) {
            Ok(validated) => {
                schemas_by_subject.insert(subject.clone(), validated);
            }
            Err(report) => {
                violations.push(ConfigViolation::SchemaInvalid {
                    subject: subject.clone(),
                    report,
                });
            }
        }
    }
}

fn parse_revision_links(
    value: &Value,
    parsed_links: &mut Vec<(usize, RevisionLink)>,
    violations: &mut Vec<ConfigViolation>,
) {
    let Some(links) = value.as_array() else {
        violations.push(ConfigViolation::RevisionLinksNotArray);
        return;
    };

    for (index, value) in links.iter().enumerate() {
        let Some(link) = parse_revision_link(value, index, violations) else {
            continue;
        };
        parsed_links.push((index, link));
    }
}

fn parse_revision_link(
    value: &Value,
    index: usize,
    violations: &mut Vec<ConfigViolation>,
) -> Option<RevisionLink> {
    let Some(object) = value.as_object() else {
        violations.push(ConfigViolation::MalformedRevisionLink {
            index,
            kind: RevisionLinkViolation::EntryNotObject,
        });
        return None;
    };

    let mut malformed = false;
    for key in object.keys() {
        if !REVISION_LINK_KEYS.contains(&key.as_str()) {
            malformed = true;
            violations.push(ConfigViolation::MalformedRevisionLink {
                index,
                kind: RevisionLinkViolation::ExtraField { field: key.clone() },
            });
        }
    }

    let from = parse_link_string(object, "from", index, violations, &mut malformed);
    let field = parse_link_string(object, "field", index, violations, &mut malformed);
    let to = parse_link_string(object, "to", index, violations, &mut malformed);

    let (Some(from), Some(field), Some(to)) = (from, field, to) else {
        return None;
    };

    for (name, value) in [("from", &from), ("to", &to)] {
        if !is_known_subject(value) {
            malformed = true;
            violations.push(ConfigViolation::MalformedRevisionLink {
                index,
                kind: RevisionLinkViolation::UnknownSubject {
                    field: name.to_owned(),
                    value: value.clone(),
                },
            });
        }
    }

    if field.is_empty() {
        malformed = true;
        violations.push(ConfigViolation::MalformedRevisionLink {
            index,
            kind: RevisionLinkViolation::EmptyField {
                field: "field".to_owned(),
            },
        });
    }

    if malformed {
        None
    } else {
        Some(RevisionLink { from, field, to })
    }
}

fn parse_link_string(
    object: &Map<String, Value>,
    name: &str,
    index: usize,
    violations: &mut Vec<ConfigViolation>,
    malformed: &mut bool,
) -> Option<String> {
    let Some(value) = object.get(name) else {
        *malformed = true;
        violations.push(ConfigViolation::MalformedRevisionLink {
            index,
            kind: RevisionLinkViolation::MissingField {
                field: name.to_owned(),
            },
        });
        return None;
    };
    let Some(value) = value.as_str() else {
        *malformed = true;
        violations.push(ConfigViolation::MalformedRevisionLink {
            index,
            kind: RevisionLinkViolation::WrongType {
                field: name.to_owned(),
            },
        });
        return None;
    };
    Some(value.to_owned())
}

fn validate_subject_metadata(
    axes_by_gate: &BTreeMap<String, BTreeMap<String, PolicyAxis>>,
    raw_schemas: &BTreeMap<String, Value>,
    parsed_links: &[(usize, RevisionLink)],
    violations: &mut Vec<ConfigViolation>,
) {
    for (gate, axes) in axes_by_gate {
        if axes.is_empty() {
            continue;
        }
        let subject = gate_subject(gate).expect("all parsed policy gates are known");
        let Some(schema) = raw_schemas.get(subject) else {
            violations.push(ConfigViolation::AxesRequireSchema {
                gate: gate.clone(),
                subject: subject.to_owned(),
            });
            continue;
        };

        let issues = schema_metadata_issues(schema);
        if !issues.is_empty() {
            violations.push(ConfigViolation::SchemaMetadataInvalid {
                subject: subject.to_owned(),
                issues,
            });
        }
    }

    for (index, link) in parsed_links {
        let mut issues = Vec::new();
        if !raw_schemas.contains_key(link.from()) {
            issues.push(LinkSchemaViolation::MissingFromSchema);
        }
        if !raw_schemas.contains_key(link.to()) {
            issues.push(LinkSchemaViolation::MissingToSchema);
        } else if !has_revision_clause(
            raw_schemas
                .get(link.to())
                .expect("checked configured to schema"),
        ) {
            issues.push(LinkSchemaViolation::ToSchemaMissingRevision);
        }

        if !issues.is_empty() {
            violations.push(ConfigViolation::LinkRequiresSchemas {
                index: *index,
                from: link.from().to_owned(),
                to: link.to().to_owned(),
                issues,
            });
        }
    }
}

fn schema_metadata_issues(schema: &Value) -> Vec<String> {
    let mut issues = Vec::new();
    let Some(schema) = schema.as_object() else {
        return vec!["top-level schema must be an object schema".to_owned()];
    };

    if schema.get("type").and_then(Value::as_str) != Some("object") {
        issues.push("top-level type must be `object`".to_owned());
    }

    let properties = schema.get("properties").and_then(Value::as_object);
    let required = schema.get("required").and_then(Value::as_array);

    if !contains_required(required, "revision") {
        issues.push("required must contain `revision`".to_owned());
    }
    if !property_has_type(properties, "revision", "string") {
        issues.push("properties.revision.type must be `string`".to_owned());
    }

    if !contains_required(required, "author") {
        issues.push("required must contain `author`".to_owned());
    }
    let author = properties
        .and_then(|properties| properties.get("author"))
        .and_then(Value::as_object);
    if author
        .and_then(|schema| schema.get("type"))
        .and_then(Value::as_str)
        != Some("object")
    {
        issues.push("properties.author.type must be `object`".to_owned());
    }

    let author_properties = author
        .and_then(|schema| schema.get("properties"))
        .and_then(Value::as_object);
    let author_required = author
        .and_then(|schema| schema.get("required"))
        .and_then(Value::as_array);

    for name in ["name", "kind"] {
        if !contains_required(author_required, name) {
            issues.push(format!("properties.author.required must contain `{name}`"));
        }
        if !property_has_type(author_properties, name, "string") {
            issues.push(format!(
                "properties.author.properties.{name}.type must be `string`"
            ));
        }
    }

    let kind_schema = author_properties
        .and_then(|properties| properties.get("kind"))
        .and_then(Value::as_object);
    if !has_author_kind_enum(kind_schema.and_then(|schema| schema.get("enum"))) {
        issues.push(
            "properties.author.properties.kind.enum must be exactly human, agent, script"
                .to_owned(),
        );
    }

    issues
}

fn has_revision_clause(schema: &Value) -> bool {
    let Some(schema) = schema.as_object() else {
        return false;
    };
    schema.get("type").and_then(Value::as_str) == Some("object")
        && contains_required(schema.get("required").and_then(Value::as_array), "revision")
        && property_has_type(
            schema.get("properties").and_then(Value::as_object),
            "revision",
            "string",
        )
}

fn contains_required(required: Option<&Vec<Value>>, name: &str) -> bool {
    required.is_some_and(|required| required.iter().any(|entry| entry.as_str() == Some(name)))
}

fn property_has_type(
    properties: Option<&Map<String, Value>>,
    name: &str,
    expected_type: &str,
) -> bool {
    properties
        .and_then(|properties| properties.get(name))
        .and_then(Value::as_object)
        .and_then(|schema| schema.get("type"))
        .and_then(Value::as_str)
        == Some(expected_type)
}

fn has_author_kind_enum(enum_value: Option<&Value>) -> bool {
    let Some(values) = enum_value.and_then(Value::as_array) else {
        return false;
    };
    let values = values
        .iter()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    values.len() == AUTHOR_KINDS.len() && AUTHOR_KINDS.iter().all(|kind| values.contains(kind))
}

fn config_violation_sort_key(violation: &ConfigViolation) -> String {
    format!("{violation}")
}

fn is_known_gate(gate: &str) -> bool {
    GATE_IDS.contains(&gate)
}

fn is_known_subject(subject: &str) -> bool {
    SUBJECT_NAMES.contains(&subject)
}

fn gate_subject(gate: &str) -> Option<&'static str> {
    match gate {
        "verify" => Some("verification.json"),
        "synthesize" => Some("report.json"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_config() -> Value {
        json!({
            "config_version": "test-1",
            "review_policies": {}
        })
    }

    fn author_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "revision": {"type": "string"},
                "author": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "kind": {
                            "type": "string",
                            "enum": ["human", "agent", "script"]
                        }
                    },
                    "required": ["name", "kind"],
                    "additionalProperties": false
                }
            },
            "required": ["revision", "author"],
            "additionalProperties": false
        })
    }

    fn revision_schema() -> Value {
        json!({
            "type": "object",
            "properties": {"revision": {"type": "string"}},
            "required": ["revision"]
        })
    }

    fn axis_config(gate: &str, schema_subject: &str, schema: Value) -> Value {
        let mut config = empty_config();
        config["review_policies"] = json!({
            gate: [{"id": "axis", "description": "desc"}]
        });
        config["artifact_schemas"] = json!({schema_subject: schema});
        config
    }

    fn classes(error: ConfigValidationError) -> Vec<&'static str> {
        error
            .violations()
            .iter()
            .map(ConfigViolation::class)
            .collect()
    }

    #[test]
    fn valid_config_parses_into_semantically_keyed_surface() {
        let mut config = axis_config("verify", "verification.json", author_schema());
        config["artifact_root"] = json!(null);
        config["extra"] = json!({"caller": [1, true]});
        config["revision_links"] = json!([
            {"from": "sources.json", "field": "brief_revision", "to": "brief.json"}
        ]);
        config["artifact_schemas"]["brief.json"] = revision_schema();
        config["artifact_schemas"]["sources.json"] = author_schema();

        let parsed = parse_initial_input(&config).expect("config should validate");
        assert_eq!(parsed.config_version(), "test-1");
        assert_eq!(parsed.artifact_root(), Some(&Value::Null));
        assert_eq!(parsed.extra(), Some(&json!({"caller": [1, true]})));
        assert!(parsed.schema("verification.json").is_some());
        assert_eq!(parsed.links_from("sources.json").len(), 1);
        assert_eq!(
            parsed.links_from("sources.json")[0].field(),
            "brief_revision"
        );
        assert_eq!(parsed.axis("verify", "axis").unwrap().required_authors(), 1);
        assert_eq!(
            parsed
                .axis_namespace()
                .get("verify")
                .expect("configured gate"),
            &BTreeSet::from(["axis".to_owned()])
        );
    }

    #[test]
    fn explicitly_empty_policies_and_missing_artifact_root_validate_clean() {
        let parsed = parse_initial_input(&empty_config()).expect("zero-obligation config is valid");
        assert!(parsed.artifact_root().is_none());
        assert!(parsed.schemas_by_subject().is_empty());
        assert!(parsed.axes_by_gate().is_empty());
    }

    #[test]
    fn extra_and_unknown_policy_fields_are_ignored() {
        let mut config = axis_config("verify", "verification.json", author_schema());
        config["extra"] = json!("opaque");
        config["review_policies"]["verify"][0]["example_prompt"] = json!({"opaque": true});
        config["review_policies"]["verify"][0]["future_field"] = json!(null);
        assert!(parse_initial_input(&config).is_ok());
    }

    #[test]
    fn unknown_top_level_key_is_malformed() {
        let mut config = empty_config();
        config["artifact_schema"] = json!({});
        let error = parse_initial_input(&config).expect_err("typoed top-level key must fail");
        assert!(classes(error).contains(&"unknown-top-level-key"));
    }

    #[test]
    fn missing_review_policies_names_shipped_configs() {
        let error = parse_initial_input(&json!({"config_version": "test-1"}))
            .expect_err("review policies are required");
        assert!(classes(error).contains(&"review-policies-missing"));
        let message = parse_initial_input(&json!({"config_version": "test-1"}))
            .expect_err("review policies are required")
            .to_string();
        assert!(message.contains("standard"));
    }

    #[test]
    fn unknown_gate_and_artifact_names_are_rejected() {
        let config = json!({
            "config_version": "test-1",
            "review_policies": {"design": []},
            "artifact_schemas": {"unknown.json": {"type": "object"}}
        });
        let error = parse_initial_input(&config).expect_err("unknown names must fail");
        let classes = classes(error);
        assert!(classes.contains(&"unknown-gate-name"));
        assert!(classes.contains(&"unknown-artifact-name"));
    }

    #[test]
    fn axes_without_schema_are_rejected() {
        let config = json!({
            "config_version": "test-1",
            "review_policies": {
                "verify": [{"id": "axis", "description": "desc"}]
            }
        });
        let error = parse_initial_input(&config).expect_err("axis needs schema");
        assert!(classes(error).contains(&"axes-require-schema"));
    }

    #[test]
    fn schema_missing_revision_and_author_metadata_is_rejected() {
        let config = axis_config("verify", "verification.json", json!({"type": "object"}));
        let error = parse_initial_input(&config).expect_err("metadata is required");
        assert!(classes(error).contains(&"schema-metadata-invalid"));
    }

    #[test]
    fn object_schema_without_metadata_on_axis_is_rejected() {
        let config = axis_config("verify", "verification.json", json!({"type": "object"}));
        let error = parse_initial_input(&config).expect_err("B2 object schema must be rejected");
        assert!(error.violations().iter().any(|violation| {
            matches!(violation, ConfigViolation::SchemaMetadataInvalid { .. })
        }));
    }

    #[test]
    fn link_without_schemas_on_both_ends_is_rejected() {
        let mut config = empty_config();
        config["revision_links"] = json!([
            {"from": "sources.json", "field": "brief_revision", "to": "brief.json"}
        ]);
        let error = parse_initial_input(&config).expect_err("links need schemas");
        assert!(classes(error).contains(&"links-require-schemas"));
    }

    #[test]
    fn link_to_schema_must_supply_revision_clause() {
        let mut config = empty_config();
        config["artifact_schemas"] = json!({
            "sources.json": revision_schema(),
            "brief.json": {"type": "object"}
        });
        config["revision_links"] = json!([
            {"from": "sources.json", "field": "brief_revision", "to": "brief.json"}
        ]);
        let error = parse_initial_input(&config).expect_err("to schema needs revision");
        assert!(error.violations().iter().any(|violation| {
            matches!(
                violation,
                ConfigViolation::LinkRequiresSchemas { issues, .. }
                    if issues.contains(&LinkSchemaViolation::ToSchemaMissingRevision)
            )
        }));
    }

    #[test]
    fn bad_required_authors_is_rejected() {
        let mut config = axis_config("verify", "verification.json", author_schema());
        config["review_policies"]["verify"][0]["required_authors"] = json!(0);
        let error = parse_initial_input(&config).expect_err("required authors must be positive");
        assert!(classes(error).contains(&"bad-required-authors"));
    }

    #[test]
    fn duplicate_axis_id_is_rejected_within_gate() {
        let mut config = axis_config("verify", "verification.json", author_schema());
        config["review_policies"]["verify"] = json!([
            {"id": "axis", "description": "one"},
            {"id": "axis", "description": "two"}
        ]);
        let error = parse_initial_input(&config).expect_err("duplicate ids must fail");
        assert!(classes(error).contains(&"duplicate-axis-id"));
    }

    #[test]
    fn bad_schema_keyword_is_rejected_by_generic_validator() {
        let mut config = empty_config();
        config["artifact_schemas"] = json!({
            "brief.json": {"type": "object", "not_a_keyword": true}
        });
        let error = parse_initial_input(&config).expect_err("unknown schema keyword must fail");
        assert!(error.violations().iter().any(|violation| {
            matches!(violation, ConfigViolation::SchemaInvalid { report, .. }
                if report.violations().iter().any(|meta| meta.rule == "unknown-keyword"))
        }));
    }

    #[test]
    fn config_version_missing_non_string_and_empty_are_distinct() {
        let missing = json!({"review_policies": {}});
        let non_string = json!({"config_version": 1, "review_policies": {}});
        let empty = json!({"config_version": "", "review_policies": {}});
        assert_eq!(
            classes(parse_initial_input(&missing).expect_err("missing version")),
            vec!["config-version-missing"]
        );
        assert_eq!(
            classes(parse_initial_input(&non_string).expect_err("non-string version")),
            vec!["config-version-not-string"]
        );
        assert_eq!(
            classes(parse_initial_input(&empty).expect_err("empty version")),
            vec!["config-version-empty"]
        );
    }

    #[test]
    fn malformed_revision_link_missing_field_is_rejected() {
        let mut config = empty_config();
        config["artifact_schemas"] = json!({
            "sources.json": revision_schema(),
            "brief.json": revision_schema()
        });
        config["revision_links"] = json!([
            {"from": "sources.json", "to": "brief.json"}
        ]);
        let error = parse_initial_input(&config).expect_err("missing link field");
        assert!(error.violations().iter().any(|violation| {
            matches!(
                violation,
                ConfigViolation::MalformedRevisionLink {
                    kind: RevisionLinkViolation::MissingField { field }, ..
                } if field == "field"
            )
        }));
    }

    #[test]
    fn malformed_revision_link_wrong_type_is_rejected() {
        let mut config = empty_config();
        config["artifact_schemas"] = json!({
            "sources.json": revision_schema(),
            "brief.json": revision_schema()
        });
        config["revision_links"] = json!([
            {"from": "sources.json", "field": 3, "to": "brief.json"}
        ]);
        let error = parse_initial_input(&config).expect_err("wrong link field type");
        assert!(error.violations().iter().any(|violation| {
            matches!(
                violation,
                ConfigViolation::MalformedRevisionLink {
                    kind: RevisionLinkViolation::WrongType { field }, ..
                } if field == "field"
            )
        }));
    }

    #[test]
    fn malformed_revision_link_extra_field_is_rejected() {
        let mut config = empty_config();
        config["artifact_schemas"] = json!({
            "sources.json": revision_schema(),
            "brief.json": revision_schema()
        });
        config["revision_links"] = json!([
            {"from": "sources.json", "field": "brief_revision", "to": "brief.json", "extra": true}
        ]);
        let error = parse_initial_input(&config).expect_err("extra link field");
        assert!(error.violations().iter().any(|violation| {
            matches!(
                violation,
                ConfigViolation::MalformedRevisionLink {
                    kind: RevisionLinkViolation::ExtraField { field }, ..
                } if field == "extra"
            )
        }));
    }

    #[test]
    fn malformed_revision_link_unknown_subject_is_rejected() {
        let mut config = empty_config();
        config["revision_links"] = json!([
            {"from": "unknown.json", "field": "brief_revision", "to": "brief.json"}
        ]);
        let error = parse_initial_input(&config).expect_err("unknown link subject");
        assert!(error.violations().iter().any(|violation| {
            matches!(
                violation,
                ConfigViolation::MalformedRevisionLink {
                    kind: RevisionLinkViolation::UnknownSubject { field, .. }, ..
                } if field == "from"
            )
        }));
    }

    #[test]
    fn malformed_config_shape_classes_are_table_driven() {
        let cases = vec![
            (
                "review policies not object",
                json!({
                    "config_version": "test-1",
                    "review_policies": []
                }),
                "review-policies-shape",
            ),
            (
                "policy axes not array",
                json!({
                    "config_version": "test-1",
                    "review_policies": {"verify": {}}
                }),
                "policy-axes-shape",
            ),
            (
                "policy entry not object",
                json!({
                    "config_version": "test-1",
                    "review_policies": {"verify": [null]}
                }),
                "policy-entry-shape",
            ),
            (
                "policy id missing",
                json!({
                    "config_version": "test-1",
                    "review_policies": {
                        "verify": [{"description": "desc"}]
                    }
                }),
                "policy-id-missing",
            ),
            (
                "policy id non-string",
                json!({
                    "config_version": "test-1",
                    "review_policies": {
                        "verify": [{"id": 1, "description": "desc"}]
                    }
                }),
                "policy-id-not-string",
            ),
            (
                "policy id empty",
                json!({
                    "config_version": "test-1",
                    "review_policies": {
                        "verify": [{"id": "", "description": "desc"}]
                    }
                }),
                "policy-id-empty",
            ),
            (
                "policy description missing",
                json!({
                    "config_version": "test-1",
                    "review_policies": {"verify": [{"id": "axis"}]}
                }),
                "policy-description-missing",
            ),
            (
                "policy description non-string",
                json!({
                    "config_version": "test-1",
                    "review_policies": {
                        "verify": [{"id": "axis", "description": false}]
                    }
                }),
                "policy-description-not-string",
            ),
            (
                "artifact schemas not object",
                json!({
                    "config_version": "test-1",
                    "review_policies": {},
                    "artifact_schemas": []
                }),
                "artifact-schemas-shape",
            ),
            (
                "revision links not array",
                json!({
                    "config_version": "test-1",
                    "review_policies": {},
                    "revision_links": {}
                }),
                "revision-links-shape",
            ),
        ];

        for (name, config, expected_class) in cases {
            let error = match parse_initial_input(&config) {
                Ok(_) => panic!("{name}: malformed config unexpectedly accepted"),
                Err(error) => error,
            };
            assert!(
                error
                    .violations()
                    .iter()
                    .any(|violation| violation.class() == expected_class),
                "{name}: expected {expected_class}, got {:?}",
                error.violations()
            );
        }
    }

    #[test]
    fn malformed_revision_link_shape_classes_are_table_driven() {
        let cases = vec![
            (
                "non-object link entry",
                json!([null]),
                RevisionLinkViolation::EntryNotObject,
            ),
            (
                "empty link field",
                json!([{
                    "from": "brief.json",
                    "field": "",
                    "to": "sources.json"
                }]),
                RevisionLinkViolation::EmptyField {
                    field: "field".to_owned(),
                },
            ),
        ];

        for (name, links, expected_kind) in cases {
            let mut config = empty_config();
            config["revision_links"] = links;
            let error = match parse_initial_input(&config) {
                Ok(_) => panic!("{name}: malformed link unexpectedly accepted"),
                Err(error) => error,
            };
            assert!(
                error.violations().iter().any(|violation| {
                    matches!(
                        violation,
                        ConfigViolation::MalformedRevisionLink { kind, .. }
                            if kind == &expected_kind
                    )
                }),
                "{name}: expected {expected_kind:?}, got {:?}",
                error.violations()
            );
        }
    }

    #[test]
    fn non_object_initial_input_is_rejected() {
        let error = parse_initial_input(&json!(null)).expect_err("root must be object");
        assert_eq!(classes(error), vec!["top-level-shape"]);
    }
}
