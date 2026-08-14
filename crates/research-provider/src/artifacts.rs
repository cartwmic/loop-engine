//! Contained artifact reads, metadata extraction, and revision-link checks.
//!
//! This module owns filesystem outcomes only.  It does not decide which
//! transition or policy requires a read, and it does not construct provider
//! responses; `gates.rs` maps these outcome classes to the wire contract.

#![allow(dead_code)]

use crate::config::RevisionLink;
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

const AUTHOR_KINDS: &[&str] = &["human", "agent", "script"];

/// Parsed artifact document whose path has passed containment checks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArtifactDocument {
    subject: String,
    path: PathBuf,
    value: Value,
}

impl ArtifactDocument {
    pub(crate) fn subject(&self) -> &str {
        &self.subject
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn value(&self) -> &Value {
        &self.value
    }
}

/// Denials caused by expected work not being ready or by an authored document
/// that is not parseable JSON.  These are policy-denial inputs, not provider
/// incapacity; T06 chooses the response code and feedback wording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactReadDeny {
    NotFound {
        subject: String,
        path: PathBuf,
    },
    Unparseable {
        subject: String,
        path: PathBuf,
        message: String,
    },
}

impl ArtifactReadDeny {
    pub(crate) fn subject(&self) -> &str {
        match self {
            Self::NotFound { subject, .. } | Self::Unparseable { subject, .. } => subject,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::NotFound { path, .. } | Self::Unparseable { path, .. } => path,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::NotFound { .. } => "work not yet authored".to_owned(),
            Self::Unparseable { message, .. } => message.clone(),
        }
    }
}

impl fmt::Display for ArtifactReadDeny {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { subject, path } => write!(
                formatter,
                "artifact `{subject}` not found at {}: work not yet authored",
                path.display()
            ),
            Self::Unparseable {
                subject,
                path,
                message,
            } => write!(
                formatter,
                "artifact `{subject}` at {} is not parseable JSON: {message}",
                path.display()
            ),
        }
    }
}

/// Environment or provider-capacity failure while preparing or reading an
/// artifact.  This is intentionally separate from `ArtifactReadDeny`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArtifactReadError {
    subject: Option<String>,
    path: Option<PathBuf>,
    message: String,
}

impl ArtifactReadError {
    fn new(
        subject: Option<impl Into<String>>,
        path: Option<PathBuf>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.map(Into::into),
            path,
            message: message.into(),
        }
    }

    pub(crate) fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ArtifactReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(subject) = &self.subject {
            write!(formatter, "artifact `{subject}`: {}", self.message)
        } else {
            formatter.write_str(&self.message)
        }
    }
}

impl std::error::Error for ArtifactReadError {}

/// Result of one contained artifact read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ArtifactReadOutcome {
    Present(ArtifactDocument),
    Deny(ArtifactReadDeny),
    EvaluationError(ArtifactReadError),
}

impl ArtifactReadOutcome {
    pub(crate) fn document(&self) -> Option<&ArtifactDocument> {
        match self {
            Self::Present(document) => Some(document),
            Self::Deny(_) | Self::EvaluationError(_) => None,
        }
    }
}

/// Read fixed artifact name beneath caller-declared `artifact_root`.
///
/// Root validation and target canonicalization happen here so callers cannot
/// accidentally turn an inaccessible root or symlink escape into a document
/// denial.  `artifact_root` is raw initial-input JSON because config parsing
/// deliberately does not inspect it.
pub(crate) fn read_artifact(artifact_root: Option<&Value>, subject: &str) -> ArtifactReadOutcome {
    let canonical_root = match canonical_root(artifact_root) {
        Ok(root) => root,
        Err(error) => return ArtifactReadOutcome::EvaluationError(error),
    };

    let target = match fixed_subject_path(&canonical_root, subject) {
        Ok(target) => target,
        Err(message) => {
            return ArtifactReadOutcome::EvaluationError(ArtifactReadError::new(
                Some(subject),
                Some(canonical_root),
                message,
            ))
        }
    };

    let canonical_target = match fs::canonicalize(&target) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ArtifactReadOutcome::Deny(ArtifactReadDeny::NotFound {
                subject: subject.to_owned(),
                path: target,
            })
        }
        Err(error) => {
            return ArtifactReadOutcome::EvaluationError(ArtifactReadError::new(
                Some(subject),
                Some(target),
                format!("could not canonicalize target: {error}"),
            ))
        }
    };

    if !is_contained(&canonical_root, &canonical_target) {
        return ArtifactReadOutcome::EvaluationError(ArtifactReadError::new(
            Some(subject),
            Some(canonical_target),
            format!(
                "canonical target escapes artifact root `{}`",
                canonical_root.display()
            ),
        ));
    }

    let content = match fs::read_to_string(&canonical_target) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ArtifactReadOutcome::Deny(ArtifactReadDeny::NotFound {
                subject: subject.to_owned(),
                path: canonical_target,
            })
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            return ArtifactReadOutcome::Deny(ArtifactReadDeny::Unparseable {
                subject: subject.to_owned(),
                path: canonical_target,
                message: format!("file is not valid UTF-8: {error}"),
            })
        }
        Err(error) => {
            return ArtifactReadOutcome::EvaluationError(ArtifactReadError::new(
                Some(subject),
                Some(canonical_target),
                format!("could not read artifact: {error}"),
            ))
        }
    };

    let value = match serde_json::from_str::<Value>(&content) {
        Ok(value) => value,
        Err(error) => {
            return ArtifactReadOutcome::Deny(ArtifactReadDeny::Unparseable {
                subject: subject.to_owned(),
                path: canonical_target,
                message: error.to_string(),
            })
        }
    };

    ArtifactReadOutcome::Present(ArtifactDocument {
        subject: subject.to_owned(),
        path: canonical_target,
        value,
    })
}

fn canonical_root(artifact_root: Option<&Value>) -> Result<PathBuf, ArtifactReadError> {
    let Some(root_value) = artifact_root else {
        return Err(ArtifactReadError::new(
            None::<String>,
            None,
            "artifact_root is required for an artifact read",
        ));
    };
    let Some(root_string) = root_value.as_str() else {
        return Err(ArtifactReadError::new(
            None::<String>,
            None,
            "artifact_root must be an absolute path string",
        ));
    };
    let root = PathBuf::from(root_string);
    if !root.is_absolute() {
        return Err(ArtifactReadError::new(
            None::<String>,
            Some(root),
            "artifact_root must be an absolute path",
        ));
    }

    let canonical = fs::canonicalize(&root).map_err(|error| {
        ArtifactReadError::new(
            None::<String>,
            Some(root.clone()),
            format!("could not canonicalize artifact_root: {error}"),
        )
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        ArtifactReadError::new(
            None::<String>,
            Some(canonical.clone()),
            format!("could not inspect artifact_root: {error}"),
        )
    })?;
    if !metadata.is_dir() {
        return Err(ArtifactReadError::new(
            None::<String>,
            Some(canonical),
            "artifact_root must name a directory",
        ));
    }
    Ok(canonical)
}

fn fixed_subject_path(root: &Path, subject: &str) -> Result<PathBuf, String> {
    let path = Path::new(subject);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(root.join(path)),
        _ => Err(format!(
            "artifact subject `{subject}` is not a fixed file name"
        )),
    }
}

fn is_contained(root: &Path, target: &Path) -> bool {
    target != root
        && target
            .strip_prefix(root)
            .map(|relative| !relative.as_os_str().is_empty())
            .unwrap_or(false)
}

/// A checked revision linkage outcome.  Read failures for the target preserve
/// the §6 deny/error classes; actual linkage mismatches are explicit
/// violations for T06 to report with schema feedback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkCheckOutcome {
    Holds,
    Violation(LinkViolation),
    ReadDenied(ArtifactReadDeny),
    EvaluationError(ArtifactReadError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkViolation {
    FromFieldMissing {
        from_subject: String,
        from_path: String,
    },
    FromFieldNotString {
        from_subject: String,
        from_path: String,
    },
    TargetRevisionMissing {
        from_subject: String,
        from_path: String,
        to_subject: String,
        to_path: String,
    },
    TargetRevisionNotString {
        from_subject: String,
        from_path: String,
        to_subject: String,
        to_path: String,
    },
    RevisionMismatch {
        from_subject: String,
        from_path: String,
        to_subject: String,
        to_path: String,
        actual: String,
        expected: String,
    },
}

impl LinkViolation {
    pub(crate) fn source_subject(&self) -> &str {
        match self {
            Self::FromFieldMissing { from_subject, .. }
            | Self::FromFieldNotString { from_subject, .. }
            | Self::TargetRevisionMissing { from_subject, .. }
            | Self::TargetRevisionNotString { from_subject, .. }
            | Self::RevisionMismatch { from_subject, .. } => from_subject,
        }
    }
}

impl fmt::Display for LinkViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FromFieldMissing {
                from_subject,
                from_path,
            } => write!(
                formatter,
                "revision link source `{from_subject}{from_path}` is missing"
            ),
            Self::FromFieldNotString {
                from_subject,
                from_path,
            } => write!(
                formatter,
                "revision link source `{from_subject}{from_path}` must be a string"
            ),
            Self::TargetRevisionMissing {
                from_subject,
                from_path,
                to_subject,
                to_path,
            } => write!(
                formatter,
                "revision link `{from_subject}{from_path}` targets missing `{to_subject}{to_path}`"
            ),
            Self::TargetRevisionNotString {
                from_subject,
                from_path,
                to_subject,
                to_path,
            } => write!(
                formatter,
                "revision link `{from_subject}{from_path}` targets non-string `{to_subject}{to_path}`"
            ),
            Self::RevisionMismatch {
                from_subject,
                from_path,
                to_subject,
                to_path,
                actual,
                expected,
            } => write!(
                formatter,
                "revision link `{from_subject}{from_path}` value `{actual}` does not match `{to_subject}{to_path}` value `{expected}`"
            ),
        }
    }
}

/// Check one configured link against the already-read source document.
pub(crate) fn check_revision_link(
    artifact_root: Option<&Value>,
    from_subject: &str,
    from_value: &Value,
    link: &RevisionLink,
) -> LinkCheckOutcome {
    let from_path = json_pointer_field(link.field());
    let Some(source_field) = from_value.get(link.field()) else {
        return LinkCheckOutcome::Violation(LinkViolation::FromFieldMissing {
            from_subject: from_subject.to_owned(),
            from_path,
        });
    };
    let Some(source_revision) = source_field.as_str() else {
        return LinkCheckOutcome::Violation(LinkViolation::FromFieldNotString {
            from_subject: from_subject.to_owned(),
            from_path,
        });
    };

    let target_subject = link.to().to_owned();
    let target_path = "/revision".to_owned();
    let target = match read_artifact(artifact_root, link.to()) {
        ArtifactReadOutcome::Present(document) => document,
        ArtifactReadOutcome::Deny(deny) => return LinkCheckOutcome::ReadDenied(deny),
        ArtifactReadOutcome::EvaluationError(error) => {
            return LinkCheckOutcome::EvaluationError(error)
        }
    };

    let Some(target_revision) = target.value().get("revision") else {
        return LinkCheckOutcome::Violation(LinkViolation::TargetRevisionMissing {
            from_subject: from_subject.to_owned(),
            from_path,
            to_subject: target_subject,
            to_path: target_path,
        });
    };
    let Some(target_revision) = target_revision.as_str() else {
        return LinkCheckOutcome::Violation(LinkViolation::TargetRevisionNotString {
            from_subject: from_subject.to_owned(),
            from_path,
            to_subject: target_subject,
            to_path: target_path,
        });
    };

    if source_revision == target_revision {
        LinkCheckOutcome::Holds
    } else {
        LinkCheckOutcome::Violation(LinkViolation::RevisionMismatch {
            from_subject: from_subject.to_owned(),
            from_path,
            to_subject: target_subject,
            to_path: target_path,
            actual: source_revision.to_owned(),
            expected: target_revision.to_owned(),
        })
    }
}

fn json_pointer_field(field: &str) -> String {
    format!("/{}", field.replace('~', "~0").replace('/', "~1"))
}

/// Author declaration extracted from a schema-valid artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorIdentity {
    name: String,
    kind: String,
}

impl AuthorIdentity {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn kind(&self) -> &str {
        &self.kind
    }
}

/// One covered document in a report's coverage manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoveredDocument {
    path: String,
    revision: String,
}

impl CoveredDocument {
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }
}

/// Coverage declaration carried by implementation and validation reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CoverageManifest {
    commit: String,
    documents: Vec<CoveredDocument>,
}

impl CoverageManifest {
    pub(crate) fn commit(&self) -> &str {
        &self.commit
    }

    pub(crate) fn documents(&self) -> &[CoveredDocument] {
        &self.documents
    }
}

/// Metadata common to authored artifacts, with report coverage when present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactMetadata {
    revision: String,
    author: AuthorIdentity,
    coverage: Option<CoverageManifest>,
}

impl ArtifactMetadata {
    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }

    pub(crate) fn author(&self) -> &AuthorIdentity {
        &self.author
    }

    pub(crate) fn coverage(&self) -> Option<&CoverageManifest> {
        self.coverage.as_ref()
    }
}

/// Extraction failure means a schema/config invariant was violated.  It is
/// intentionally not another read outcome class; callers may surface it as
/// an internal/evaluation error rather than pretending it is missing work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MetadataInvariantError {
    subject: String,
    path: String,
    message: String,
}

impl MetadataInvariantError {
    pub(crate) fn subject(&self) -> &str {
        &self.subject
    }

    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MetadataInvariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "artifact `{}` violates metadata invariant at {}: {}",
            self.subject, self.path, self.message
        )
    }
}

impl std::error::Error for MetadataInvariantError {}

/// Extract revision, author, and optional report coverage from a schema-valid
/// artifact value.  The schema guarantees these fields; failure is therefore
/// an invariant failure, not a user-document denial class.
pub(crate) fn extract_metadata(
    subject: &str,
    value: &Value,
) -> Result<ArtifactMetadata, MetadataInvariantError> {
    let object = value
        .as_object()
        .ok_or_else(|| metadata_error(subject, "/", "artifact metadata requires an object"))?;

    let revision = non_empty_string(object.get("revision"), subject, "/revision")?;

    let author_value = object
        .get("author")
        .ok_or_else(|| metadata_error(subject, "/author", "required author object is missing"))?;
    let author_object = author_value
        .as_object()
        .ok_or_else(|| metadata_error(subject, "/author", "author must be an object"))?;
    let name = non_empty_string(author_object.get("name"), subject, "/author/name")?;
    let kind = non_empty_string(author_object.get("kind"), subject, "/author/kind")?;
    if !AUTHOR_KINDS.contains(&kind.as_str()) {
        return Err(metadata_error(
            subject,
            "/author/kind",
            "author kind must be one of human, agent, script",
        ));
    }

    let coverage = if is_report_subject(subject) {
        match object.get("coverage") {
            Some(value) => Some(extract_coverage(subject, value)?),
            None => {
                return Err(metadata_error(
                    subject,
                    "/coverage",
                    "report requires coverage manifest",
                ))
            }
        }
    } else {
        None
    };

    Ok(ArtifactMetadata {
        revision,
        author: AuthorIdentity { name, kind },
        coverage,
    })
}

fn extract_coverage(
    subject: &str,
    value: &Value,
) -> Result<CoverageManifest, MetadataInvariantError> {
    let object = value
        .as_object()
        .ok_or_else(|| metadata_error(subject, "/coverage", "coverage must be an object"))?;
    let commit = non_empty_string(object.get("commit"), subject, "/coverage/commit")?;
    let documents_value = object.get("documents").ok_or_else(|| {
        metadata_error(
            subject,
            "/coverage/documents",
            "coverage documents are missing",
        )
    })?;
    let documents = documents_value.as_array().ok_or_else(|| {
        metadata_error(
            subject,
            "/coverage/documents",
            "coverage documents must be an array",
        )
    })?;

    let mut parsed = Vec::with_capacity(documents.len());
    for (index, document) in documents.iter().enumerate() {
        let path = format!("/coverage/documents/{index}");
        let object = document
            .as_object()
            .ok_or_else(|| metadata_error(subject, &path, "covered document must be an object"))?;
        let document_path = non_empty_string(object.get("path"), subject, &format!("{path}/path"))?;
        let revision =
            non_empty_string(object.get("revision"), subject, &format!("{path}/revision"))?;
        parsed.push(CoveredDocument {
            path: document_path,
            revision,
        });
    }

    Ok(CoverageManifest {
        commit,
        documents: parsed,
    })
}

fn non_empty_string(
    value: Option<&Value>,
    subject: &str,
    path: &str,
) -> Result<String, MetadataInvariantError> {
    let Some(value) = value else {
        return Err(metadata_error(
            subject,
            path,
            "required non-empty string is missing",
        ));
    };
    let Some(value) = value.as_str() else {
        return Err(metadata_error(subject, path, "value must be a string"));
    };
    if value.is_empty() {
        return Err(metadata_error(subject, path, "value must not be empty"));
    }
    Ok(value.to_owned())
}

fn metadata_error(subject: &str, path: &str, message: impl Into<String>) -> MetadataInvariantError {
    MetadataInvariantError {
        subject: subject.to_owned(),
        path: path.to_owned(),
        message: message.into(),
    }
}

fn is_report_subject(_subject: &str) -> bool {
    // Research artifacts do not carry a coverage manifest.
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_initial_input;
    use serde_json::json;
    use std::fs::{self, File};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let suffix = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "loop-engine-research-artifacts-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create temporary artifact root");
            Self { path }
        }

        fn path_value(&self) -> Value {
            json!(self.path.to_string_lossy().to_string())
        }

        fn write_json(&self, subject: &str, value: &Value) {
            fs::write(self.path.join(subject), serde_json::to_vec(value).unwrap())
                .expect("write artifact");
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn assert_eval_error(outcome: ArtifactReadOutcome) {
        assert!(
            matches!(outcome, ArtifactReadOutcome::EvaluationError(_)),
            "expected evaluation error, got {outcome:?}"
        );
    }

    fn assert_deny(outcome: ArtifactReadOutcome) -> ArtifactReadDeny {
        match outcome {
            ArtifactReadOutcome::Deny(deny) => deny,
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[test]
    fn reads_valid_json_under_root() {
        let root = TestDir::new();
        root.write_json("brief.json", &json!({"revision": "1"}));

        let outcome = read_artifact(Some(&root.path_value()), "brief.json");
        let ArtifactReadOutcome::Present(document) = outcome else {
            panic!("expected present artifact");
        };
        assert_eq!(document.subject(), "brief.json");
        assert_eq!(document.value(), &json!({"revision": "1"}));
        let canonical_root = fs::canonicalize(&root.path).unwrap();
        assert!(document.path().starts_with(canonical_root));
    }

    #[test]
    fn missing_artifact_is_deny_not_found() {
        let root = TestDir::new();
        let deny = assert_deny(read_artifact(Some(&root.path_value()), "brief.json"));
        assert!(matches!(deny, ArtifactReadDeny::NotFound { .. }));
        assert_eq!(deny.subject(), "brief.json");
    }

    #[test]
    fn unparseable_json_is_deny() {
        let root = TestDir::new();
        fs::write(root.path.join("brief.json"), b"not json").unwrap();
        let deny = assert_deny(read_artifact(Some(&root.path_value()), "brief.json"));
        assert!(matches!(deny, ArtifactReadDeny::Unparseable { .. }));
    }

    #[test]
    fn missing_relative_and_non_string_roots_are_evaluation_errors() {
        let root = TestDir::new();
        let missing = json!(root.path.join("missing-root").to_string_lossy().to_string());
        assert_eval_error(read_artifact(Some(&missing), "brief.json"));
        assert_eval_error(read_artifact(Some(&json!("relative-root")), "brief.json"));
        assert_eval_error(read_artifact(Some(&json!(42)), "brief.json"));
        assert_eval_error(read_artifact(None, "brief.json"));
    }

    #[test]
    fn file_root_and_directory_target_are_evaluation_errors() {
        let root = TestDir::new();
        let file_root = root.path.join("root-file");
        File::create(&file_root).unwrap();
        let file_root_value = json!(file_root.to_string_lossy().to_string());
        assert_eval_error(read_artifact(Some(&file_root_value), "brief.json"));

        fs::create_dir(root.path.join("brief.json")).unwrap();
        assert_eval_error(read_artifact(Some(&root.path_value()), "brief.json"));
    }

    #[test]
    fn invalid_subject_path_is_evaluation_error() {
        let root = TestDir::new();
        assert_eval_error(read_artifact(Some(&root.path_value()), "../brief.json"));
        assert_eval_error(read_artifact(Some(&root.path_value()), "nested/brief.json"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_evaluation_error() {
        use std::os::unix::fs::symlink;

        let root = TestDir::new();
        let outside = TestDir::new();
        outside.write_json("outside.json", &json!({"revision": "outside"}));
        symlink(
            outside.path.join("outside.json"),
            root.path.join("brief.json"),
        )
        .unwrap();

        assert_eval_error(read_artifact(Some(&root.path_value()), "brief.json"));
    }

    #[cfg(unix)]
    #[test]
    fn permission_denied_is_evaluation_error() {
        use std::os::unix::fs::PermissionsExt;

        let root = TestDir::new();
        root.write_json("brief.json", &json!({"revision": "1"}));
        let path = root.path.join("brief.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let outcome = read_artifact(Some(&root.path_value()), "brief.json");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        // A privileged test runner can still read mode-000 files.  In that
        // environment the branch is untestable, but must not make CI flaky.
        if !matches!(outcome, ArtifactReadOutcome::Present(_)) {
            assert_eval_error(outcome);
        }
    }

    fn link_config() -> crate::config::ValidatedConfig {
        let metadata_schema = json!({
            "type": "object",
            "properties": {"revision": {"type": "string"}},
            "required": ["revision"],
            "additionalProperties": false
        });
        parse_initial_input(&json!({
            "config_version": "test-1",
            "review_policies": {},
            "artifact_schemas": {
                "sources.json": metadata_schema,
                "brief.json": metadata_schema
            },
            "revision_links": [{
                "from": "sources.json",
                "field": "brief_revision",
                "to": "brief.json"
            }]
        }))
        .expect("valid link config")
    }

    #[test]
    fn revision_link_passes_when_source_field_matches_target_revision() {
        let root = TestDir::new();
        root.write_json("brief.json", &json!({"revision": "r1"}));
        let config = link_config();
        let link = &config.links_from("sources.json")[0];

        let result = check_revision_link(
            Some(&root.path_value()),
            "sources.json",
            &json!({"brief_revision": "r1"}),
            link,
        );
        assert_eq!(result, LinkCheckOutcome::Holds);
    }

    #[test]
    fn revision_link_reports_mismatch() {
        let root = TestDir::new();
        root.write_json("brief.json", &json!({"revision": "r2"}));
        let config = link_config();
        let link = &config.links_from("sources.json")[0];

        let result = check_revision_link(
            Some(&root.path_value()),
            "sources.json",
            &json!({"brief_revision": "r1"}),
            link,
        );
        let LinkCheckOutcome::Violation(LinkViolation::RevisionMismatch {
            from_subject,
            from_path,
            to_subject,
            to_path,
            actual,
            expected,
        }) = result
        else {
            panic!("expected revision mismatch, got {result:?}");
        };
        assert_eq!(from_subject, "sources.json");
        assert_eq!(from_path, "/brief_revision");
        assert_eq!(to_subject, "brief.json");
        assert_eq!(to_path, "/revision");
        assert_eq!(actual, "r1");
        assert_eq!(expected, "r2");
    }

    #[test]
    fn revision_link_reports_missing_or_non_string_source_field() {
        let root = TestDir::new();
        root.write_json("brief.json", &json!({"revision": "r1"}));
        let config = link_config();
        let link = &config.links_from("sources.json")[0];

        let missing =
            check_revision_link(Some(&root.path_value()), "sources.json", &json!({}), link);
        assert!(matches!(
            missing,
            LinkCheckOutcome::Violation(LinkViolation::FromFieldMissing { .. })
        ));

        let non_string = check_revision_link(
            Some(&root.path_value()),
            "sources.json",
            &json!({"brief_revision": 1}),
            link,
        );
        assert!(matches!(
            non_string,
            LinkCheckOutcome::Violation(LinkViolation::FromFieldNotString { .. })
        ));
    }

    #[test]
    fn revision_link_reports_missing_or_non_string_target_revision() {
        let root = TestDir::new();
        let config = link_config();
        let link = &config.links_from("sources.json")[0];

        root.write_json("brief.json", &json!({"other": "r1"}));
        let missing = check_revision_link(
            Some(&root.path_value()),
            "sources.json",
            &json!({"brief_revision": "r1"}),
            link,
        );
        assert!(matches!(
            missing,
            LinkCheckOutcome::Violation(LinkViolation::TargetRevisionMissing {
                ref from_subject,
                ref from_path,
                ref to_subject,
                ref to_path,
            }) if from_subject == "sources.json"
                && from_path == "/brief_revision"
                && to_subject == "brief.json"
                && to_path == "/revision"
        ));

        root.write_json("brief.json", &json!({"revision": 1}));
        let non_string = check_revision_link(
            Some(&root.path_value()),
            "sources.json",
            &json!({"brief_revision": "r1"}),
            link,
        );
        assert!(matches!(
            non_string,
            LinkCheckOutcome::Violation(LinkViolation::TargetRevisionNotString { .. })
        ));
    }

    #[test]
    fn extraction_round_trip_reads_revision_and_author() {
        let metadata = extract_metadata(
            "brief.json",
            &json!({
                "revision": "brief-1",
                "author": {"name": "cartwmic", "kind": "human"}
            }),
        )
        .expect("extract metadata");

        assert_eq!(metadata.revision(), "brief-1");
        assert_eq!(metadata.author().name(), "cartwmic");
        assert_eq!(metadata.author().kind(), "human");
        assert!(metadata.coverage().is_none());
    }

    #[test]
    fn extra_coverage_field_is_ignored_during_metadata_extraction() {
        let metadata = extract_metadata(
            "sources.json",
            &json!({
                "revision": "sources-1",
                "author": {"name": "cartwmic", "kind": "human"},
                "brief_revision": "brief-1",
                "coverage": [{
                    "acceptance": "A visible result exists.",
                    "delivered_by": "component"
                }]
            }),
        )
        .expect("schema-valid sources metadata");

        assert_eq!(metadata.revision(), "sources-1");
        assert_eq!(metadata.author().name(), "cartwmic");
        assert_eq!(metadata.author().kind(), "human");
        assert!(metadata.coverage().is_none());
    }

    #[test]
    fn report_coverage_field_is_not_required_for_research_subjects() {
        let metadata = extract_metadata(
            "report.json",
            &json!({
                "revision": "report-1",
                "author": {"name": "cartwmic", "kind": "human"},
                "coverage": []
            }),
        )
        .expect("research report metadata does not require coverage");
        assert_eq!(metadata.revision(), "report-1");
        assert!(metadata.coverage().is_none());
    }

    #[test]
    fn artifact_read_does_not_write() {
        let root = TestDir::new();
        root.write_json("brief.json", &json!({"revision": "1"}));
        let before = fs::read(root.path.join("brief.json")).unwrap();
        let _ = read_artifact(Some(&root.path_value()), "brief.json");
        let after = fs::read(root.path.join("brief.json")).unwrap();
        assert_eq!(before, after);
    }
}
