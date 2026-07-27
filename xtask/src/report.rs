//! Canonical validation evidence and immutable Git-common-directory storage.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::config::{BindingDigests, SemanticTopology};
use crate::git::Repository;
pub use crate::publication_input::{InputEvidence, RejectionCode, UpdateTuple};
use crate::publication_input::{ParsedUpdateDisposition, decode_input_evidence, parse_updates};
use crate::quality::{CandidateBinding, DeterministicPhase, DeterministicResult};
use crate::semantic_judge::{
    NormalizedResult, SemanticDisposition, SemanticResult, SemanticStatus,
};

pub const SCHEMA_VERSION: u32 = 1;
const APPROVAL_RETRIES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedDisposition {
    Pass,
    DeterministicBlock,
    SemanticBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationRecord {
    pub schema_version: u32,
    pub base_revision: String,
    pub candidate_revision: String,
    pub candidate_tree: String,
    pub manifest_digest: String,
    pub rubric_digests: BTreeMap<String, String>,
    pub semantic_topology: SemanticTopology,
    pub deterministic_results: DeterministicResult,
    pub axis_results: Vec<NormalizedResult>,
    pub coherence_result: Option<NormalizedResult>,
    pub derived_disposition: DerivedDisposition,
}

impl EvaluationRecord {
    pub fn new(
        deterministic_results: DeterministicResult,
        semantic_results: Option<SemanticResult>,
        binding: &BindingDigests,
    ) -> Result<Self> {
        let CandidateBinding {
            base_revision,
            candidate_revision,
            candidate_tree,
        } = deterministic_results.binding.clone();
        let rubric_digests = binding
            .rubric_digests()
            .iter()
            .map(|(path, digest)| {
                path.to_str()
                    .map(|path| (path.to_owned(), digest.clone()))
                    .ok_or_else(|| anyhow::anyhow!("rubric path is not UTF-8: {}", path.display()))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        let semantic_topology = binding
            .semantic_topology()
            .context("evaluation binding requires semantic topology")?
            .clone();

        let (axis_results, coherence_result, derived_disposition) =
            match (deterministic_results.passed(), semantic_results) {
                (false, None) => (Vec::new(), None, DerivedDisposition::DeterministicBlock),
                (false, Some(_)) => bail!("semantic evidence cannot follow deterministic failure"),
                (true, None) => bail!("passing deterministic evidence requires semantic evidence"),
                (true, Some(semantic)) => {
                    require_same_binding(&deterministic_results.binding, &semantic.binding)?;
                    let disposition = match semantic.disposition {
                        SemanticDisposition::Pass => DerivedDisposition::Pass,
                        SemanticDisposition::SemanticBlock => DerivedDisposition::SemanticBlock,
                    };
                    (semantic.axes, Some(semantic.coherence), disposition)
                }
            };

        let record = Self {
            schema_version: SCHEMA_VERSION,
            base_revision,
            candidate_revision,
            candidate_tree,
            manifest_digest: binding.manifest_digest().to_owned(),
            rubric_digests,
            semantic_topology,
            deterministic_results,
            axis_results,
            coherence_result,
            derived_disposition,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            bail!(
                "unsupported evaluation schema_version {}",
                self.schema_version
            );
        }
        validate_object_id(&self.base_revision, "base_revision")?;
        validate_object_id(&self.candidate_revision, "candidate_revision")?;
        validate_object_id(&self.candidate_tree, "candidate_tree")?;
        validate_digest(&self.manifest_digest, "manifest_digest")?;
        if self.rubric_digests.is_empty() {
            bail!("evaluation rubric_digests must not be empty");
        }
        for (path, digest) in &self.rubric_digests {
            validate_repository_path(path, "rubric_digests key")?;
            validate_digest(digest, "rubric digest")?;
        }
        validate_topology(&self.semantic_topology, &self.rubric_digests)?;
        if self.deterministic_results.phase != DeterministicPhase::Publication {
            bail!("evaluation deterministic phase must be publication");
        }
        require_record_binding(self, &self.deterministic_results.binding)?;

        if !self.deterministic_results.passed() {
            if self.derived_disposition != DerivedDisposition::DeterministicBlock
                || !self.axis_results.is_empty()
                || self.coherence_result.is_some()
            {
                bail!("deterministic-block evaluation has invalid semantic nullability");
            }
            return Ok(());
        }

        let expected_axis_ids = self
            .semantic_topology
            .axes
            .iter()
            .map(|axis| axis.id.as_str())
            .collect::<Vec<_>>();
        let result_axis_ids = self
            .axis_results
            .iter()
            .map(|axis| axis.id.as_str())
            .collect::<Vec<_>>();
        if result_axis_ids != expected_axis_ids {
            bail!("semantic result IDs do not exactly match stored ordered topology");
        }
        let coherence = self
            .coherence_result
            .as_ref()
            .context("semantic evaluation requires coherence_result")?;
        if coherence.id != self.semantic_topology.coherence.id {
            bail!("coherence result ID does not match stored topology");
        }
        let all_pass = self
            .axis_results
            .iter()
            .chain(std::iter::once(coherence))
            .all(|result| result.status == SemanticStatus::Pass);
        let expected = if all_pass {
            DerivedDisposition::Pass
        } else {
            DerivedDisposition::SemanticBlock
        };
        if self.derived_disposition != expected {
            bail!("evaluation derived_disposition does not match semantic evidence");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRecord {
    pub schema_version: u32,
    pub approval_id: String,
    pub report_digest: String,
    pub base_revision: String,
    pub candidate_revision: String,
    pub candidate_tree: String,
    pub manifest_digest: String,
    pub rubric_digests: BTreeMap<String, String>,
    pub semantic_topology: SemanticTopology,
    pub reason: String,
    pub created_at: String,
}

impl ApprovalRecord {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            bail!(
                "unsupported approval schema_version {}",
                self.schema_version
            );
        }
        let id = Uuid::parse_str(&self.approval_id).context("approval_id is not a UUID")?;
        if id.get_version_num() != 7 {
            bail!("approval_id must be UUIDv7");
        }
        validate_digest(&self.report_digest, "report_digest")?;
        validate_object_id(&self.base_revision, "base_revision")?;
        validate_object_id(&self.candidate_revision, "candidate_revision")?;
        validate_object_id(&self.candidate_tree, "candidate_tree")?;
        validate_digest(&self.manifest_digest, "manifest_digest")?;
        if self.rubric_digests.is_empty() {
            bail!("approval rubric_digests must not be empty");
        }
        for (path, digest) in &self.rubric_digests {
            validate_repository_path(path, "rubric_digests key")?;
            validate_digest(digest, "rubric digest")?;
        }
        validate_topology(&self.semantic_topology, &self.rubric_digests)?;
        if self.reason.trim().is_empty() {
            bail!("approval reason must be non-empty");
        }
        OffsetDateTime::parse(&self.created_at, &Rfc3339)
            .context("approval created_at must be RFC3339")?;
        Ok(())
    }

    pub fn matches_evaluation(&self, report_digest: &str, report: &EvaluationRecord) -> bool {
        self.report_digest == report_digest
            && self.base_revision == report.base_revision
            && self.candidate_revision == report.candidate_revision
            && self.candidate_tree == report.candidate_tree
            && self.manifest_digest == report.manifest_digest
            && self.rubric_digests == report.rubric_digests
            && self.semantic_topology == report.semantic_topology
            && report.derived_disposition == DerivedDisposition::SemanticBlock
    }

    pub fn matches_binding(
        &self,
        base_revision: &str,
        candidate_revision: &str,
        candidate_tree: &str,
        manifest_digest: &str,
        rubric_digests: &BTreeMap<String, String>,
        semantic_topology: &SemanticTopology,
    ) -> bool {
        self.base_revision == base_revision
            && self.candidate_revision == candidate_revision
            && self.candidate_tree == candidate_tree
            && self.manifest_digest == manifest_digest
            && &self.rubric_digests == rubric_digests
            && &self.semantic_topology == semantic_topology
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    Content,
    DeletionOnly,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    GitUpdateLines,
    CiPushEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDecision {
    Pass,
    Approved,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationAttemptRecord {
    pub schema_version: u32,
    pub update_kind: UpdateKind,
    pub input_kind: InputKind,
    pub input_evidence: InputEvidence,
    pub updates: Vec<UpdateTuple>,
    pub rejection_code: Option<RejectionCode>,
    pub base_revision: Option<String>,
    pub candidate_revision: Option<String>,
    pub candidate_tree: Option<String>,
    pub manifest_digest: Option<String>,
    pub rubric_digests: Option<BTreeMap<String, String>>,
    pub fresh_deterministic_results: Vec<DeterministicResult>,
    pub evaluation_report_digest: Option<String>,
    pub approval_digest: Option<String>,
    pub derived_disposition: DerivedDisposition,
    pub gate_decision: GateDecision,
    pub created_at: String,
}

impl PublicationAttemptRecord {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            bail!("unsupported attempt schema_version {}", self.schema_version);
        }
        let input = decode_input_evidence(&self.input_evidence)
            .context("attempt input_evidence is not lossless canonical evidence")?;
        if self.input_kind == InputKind::GitUpdateLines {
            self.validate_git_update_classification(&input)?;
        }
        OffsetDateTime::parse(&self.created_at, &Rfc3339)
            .context("attempt created_at must be RFC3339")?;
        match self.update_kind {
            UpdateKind::Content => {
                if self.rejection_code.is_some()
                    || self.updates.is_empty()
                    || self.base_revision.is_none()
                    || self.candidate_revision.is_none()
                    || self.candidate_tree.is_none()
                    || self.manifest_digest.is_none()
                    || self.rubric_digests.is_none()
                    || self.fresh_deterministic_results.len() != 1
                    || self.evaluation_report_digest.is_none()
                {
                    bail!("content attempt has invalid nullability");
                }
                validate_object_id(
                    self.base_revision.as_deref().unwrap_or_default(),
                    "base_revision",
                )?;
                validate_object_id(
                    self.candidate_revision.as_deref().unwrap_or_default(),
                    "candidate_revision",
                )?;
                validate_object_id(
                    self.candidate_tree.as_deref().unwrap_or_default(),
                    "candidate_tree",
                )?;
                validate_digest(
                    self.manifest_digest.as_deref().unwrap_or_default(),
                    "manifest_digest",
                )?;
                validate_digest(
                    self.evaluation_report_digest.as_deref().unwrap_or_default(),
                    "evaluation_report_digest",
                )?;
                let rubrics = self
                    .rubric_digests
                    .as_ref()
                    .context("content rubric digests missing")?;
                if rubrics.is_empty() {
                    bail!("content attempt rubric_digests must not be empty");
                }
                for (path, digest) in rubrics {
                    validate_repository_path(path, "rubric_digests key")?;
                    validate_digest(digest, "rubric digest")?;
                }
                if let Some(digest) = &self.approval_digest {
                    validate_digest(digest, "approval_digest")?;
                }
                let fresh = &self.fresh_deterministic_results[0];
                if fresh.phase != DeterministicPhase::Publication
                    || fresh.binding.base_revision
                        != self.base_revision.as_deref().unwrap_or_default()
                    || fresh.binding.candidate_revision
                        != self.candidate_revision.as_deref().unwrap_or_default()
                    || fresh.binding.candidate_tree
                        != self.candidate_tree.as_deref().unwrap_or_default()
                {
                    bail!("fresh deterministic evidence does not match content binding");
                }
                if fresh.passed()
                    == (self.derived_disposition == DerivedDisposition::DeterministicBlock)
                {
                    bail!("fresh deterministic pass/fail contradicts attempt derived_disposition");
                }
                let valid_gate = matches!(
                    (
                        self.derived_disposition,
                        self.gate_decision,
                        self.approval_digest.is_some()
                    ),
                    (DerivedDisposition::Pass, GateDecision::Pass, false)
                        | (
                            DerivedDisposition::DeterministicBlock,
                            GateDecision::Block,
                            false
                        )
                        | (
                            DerivedDisposition::SemanticBlock,
                            GateDecision::Block,
                            false
                        )
                        | (
                            DerivedDisposition::SemanticBlock,
                            GateDecision::Approved,
                            true
                        )
                );
                if !valid_gate {
                    bail!("content attempt gate decision is inconsistent");
                }
            }
            UpdateKind::DeletionOnly => {
                if self.rejection_code.is_some()
                    || any_content_binding(self)
                    || !self.fresh_deterministic_results.is_empty()
                    || self.derived_disposition != DerivedDisposition::Pass
                    || self.gate_decision != GateDecision::Pass
                {
                    bail!("deletion-only attempt has invalid nullability or disposition");
                }
            }
            UpdateKind::Rejected => {
                let rejection_code = self
                    .rejection_code
                    .context("rejected attempt requires rejection_code")?;
                if (matches!(
                    rejection_code,
                    RejectionCode::InvalidUpdateShape | RejectionCode::MultipleContentTips
                ) && self.updates.is_empty())
                    || (matches!(
                        rejection_code,
                        RejectionCode::MalformedUpdateInput | RejectionCode::MalformedCiEvent
                    ) && !self.updates.is_empty())
                    || any_content_binding(self)
                    || !self.fresh_deterministic_results.is_empty()
                    || self.derived_disposition != DerivedDisposition::DeterministicBlock
                    || self.gate_decision != GateDecision::Block
                {
                    bail!("rejected attempt has invalid nullability or disposition");
                }
            }
        }
        Ok(())
    }

    fn validate_git_update_classification(&self, input: &[u8]) -> Result<()> {
        let parsed = parse_updates(input);
        if parsed.updates != self.updates {
            bail!("attempt updates do not match exact canonical Git input tuples");
        }
        let (update_kind, rejection_code) = match parsed.disposition {
            ParsedUpdateDisposition::Content(_) => (UpdateKind::Content, None),
            ParsedUpdateDisposition::DeletionOnly => (UpdateKind::DeletionOnly, None),
            ParsedUpdateDisposition::Rejected(code) => (UpdateKind::Rejected, Some(code)),
        };
        if self.update_kind != update_kind || self.rejection_code != rejection_code {
            bail!("attempt Git update classification or rejection code is inconsistent");
        }
        Ok(())
    }
}

fn any_content_binding(record: &PublicationAttemptRecord) -> bool {
    record.base_revision.is_some()
        || record.candidate_revision.is_some()
        || record.candidate_tree.is_some()
        || record.manifest_digest.is_some()
        || record.rubric_digests.is_some()
        || record.evaluation_report_digest.is_some()
        || record.approval_digest.is_some()
}

#[derive(Debug, Clone)]
pub struct Store {
    common_directory: PathBuf,
}

impl Store {
    pub fn open(repository_path: &Path) -> Result<Self> {
        let repository = Repository::resolve(repository_path)?;
        Ok(Self::from_repository(&repository))
    }

    pub fn from_repository(repository: &Repository) -> Self {
        Self::from_common_directory(repository.git_common_directory())
    }

    pub fn from_common_directory(common_directory: &Path) -> Self {
        Self {
            common_directory: common_directory.to_owned(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.common_directory.join("loop-engine/validation/v1")
    }

    pub fn write_evaluation(&self, record: &EvaluationRecord) -> Result<String> {
        record.validate()?;
        let bytes = canonical_json(record)?;
        let digest = sha256_hex(&bytes);
        let path = self.report_path(&digest);
        write_immutable(&path, &bytes, true)?;
        Ok(digest)
    }

    pub fn read_evaluation(&self, digest: &str) -> Result<EvaluationRecord> {
        let record: EvaluationRecord = read_record(&self.report_path_checked(digest)?, digest)?;
        record.validate()?;
        Ok(record)
    }

    pub fn approve(&self, report_digest: &str, reason: &str) -> Result<(String, ApprovalRecord)> {
        if reason.trim().is_empty() {
            bail!("approval reason must be non-empty");
        }
        let report = self.read_evaluation(report_digest)?;
        if report.derived_disposition != DerivedDisposition::SemanticBlock {
            bail!("only a verified semantic_block evaluation can be approved");
        }
        let directory = self.approval_directory(report_digest)?;
        // Fail closed on corrupt existing evidence before adding another record.
        let approval_ids = self
            .approvals_for_report(report_digest)?
            .into_iter()
            .map(|(_, approval)| approval.approval_id)
            .collect::<BTreeSet<_>>();
        for _ in 0..APPROVAL_RETRIES {
            let approval = ApprovalRecord {
                schema_version: SCHEMA_VERSION,
                approval_id: Uuid::now_v7().to_string(),
                report_digest: report_digest.to_owned(),
                base_revision: report.base_revision.clone(),
                candidate_revision: report.candidate_revision.clone(),
                candidate_tree: report.candidate_tree.clone(),
                manifest_digest: report.manifest_digest.clone(),
                rubric_digests: report.rubric_digests.clone(),
                semantic_topology: report.semantic_topology.clone(),
                reason: reason.to_owned(),
                created_at: now_rfc3339()?,
            };
            approval.validate()?;
            if approval_ids.contains(&approval.approval_id) {
                continue;
            }
            let bytes = canonical_json(&approval)?;
            let digest = sha256_hex(&bytes);
            let path = directory.join(format!("{digest}.json"));
            match write_immutable(&path, &bytes, false)? {
                WriteDisposition::Created => return Ok((digest, approval)),
                WriteDisposition::AlreadyExists => continue,
            }
        }
        bail!("failed to create unique approval after {APPROVAL_RETRIES} attempts")
    }

    pub fn read_approval(&self, report_digest: &str, digest: &str) -> Result<ApprovalRecord> {
        let directory = self.approval_directory(report_digest)?;
        let record: ApprovalRecord = read_record(&record_path(&directory, digest)?, digest)?;
        record.validate()?;
        let report = self.read_evaluation(report_digest)?;
        if !record.matches_evaluation(report_digest, &report) {
            bail!("approval binding does not match referenced evaluation");
        }
        Ok(record)
    }

    /// Read every approval for one report, failing closed on any corrupt record.
    pub fn approvals_for_report(
        &self,
        report_digest: &str,
    ) -> Result<Vec<(String, ApprovalRecord)>> {
        let directory = self.approval_directory(report_digest)?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("failed to inspect approval directory"),
        };
        let mut digests = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                bail!("unexpected file in approval directory: {}", path.display());
            }
            let digest = path
                .file_stem()
                .and_then(|value| value.to_str())
                .context("approval filename is not UTF-8")?
                .to_owned();
            validate_digest(&digest, "approval filename digest")?;
            digests.push(digest);
        }
        digests.sort();
        digests
            .into_iter()
            .map(|digest| {
                self.read_approval(report_digest, &digest)
                    .map(|record| (digest, record))
            })
            .collect()
    }

    /// Select newest exact matching approval; digest breaks timestamp ties ascending.
    #[allow(clippy::too_many_arguments)]
    pub fn select_approval(
        &self,
        report_digest: &str,
        base_revision: &str,
        candidate_revision: &str,
        candidate_tree: &str,
        manifest_digest: &str,
        rubric_digests: &BTreeMap<String, String>,
        semantic_topology: &SemanticTopology,
    ) -> Result<Option<(String, ApprovalRecord)>> {
        let mut matches = self
            .approvals_for_report(report_digest)?
            .into_iter()
            .filter(|(_, approval)| {
                approval.matches_binding(
                    base_revision,
                    candidate_revision,
                    candidate_tree,
                    manifest_digest,
                    rubric_digests,
                    semantic_topology,
                )
            })
            .map(|(digest, approval)| {
                let created = OffsetDateTime::parse(&approval.created_at, &Rfc3339)
                    .context("approval created_at must be RFC3339")?;
                Ok((created, digest, approval))
            })
            .collect::<Result<Vec<_>>>()?;
        matches.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        Ok(matches
            .into_iter()
            .next()
            .map(|(_, digest, approval)| (digest, approval)))
    }

    /// Select newest approval and verified semantic-block evaluation matching one binding.
    #[allow(clippy::too_many_arguments)]
    pub fn select_approved_evaluation(
        &self,
        base_revision: &str,
        candidate_revision: &str,
        candidate_tree: &str,
        manifest_digest: &str,
        rubric_digests: &BTreeMap<String, String>,
        semantic_topology: &SemanticTopology,
    ) -> Result<Option<(String, EvaluationRecord, String, ApprovalRecord)>> {
        let directory = self.root().join("reports");
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to inspect evaluation directory"),
        };
        let mut report_digests = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                bail!(
                    "unexpected file in evaluation directory: {}",
                    path.display()
                );
            }
            let digest = path
                .file_stem()
                .and_then(|value| value.to_str())
                .context("evaluation filename is not UTF-8")?
                .to_owned();
            validate_digest(&digest, "evaluation filename digest")?;
            report_digests.push(digest);
        }
        report_digests.sort();

        let mut matches = Vec::new();
        for report_digest in report_digests {
            let evaluation = self.read_evaluation(&report_digest)?;
            if evaluation.derived_disposition != DerivedDisposition::SemanticBlock
                || evaluation.base_revision != base_revision
                || evaluation.candidate_revision != candidate_revision
                || evaluation.candidate_tree != candidate_tree
                || evaluation.manifest_digest != manifest_digest
                || &evaluation.rubric_digests != rubric_digests
                || &evaluation.semantic_topology != semantic_topology
            {
                continue;
            }
            if let Some((approval_digest, approval)) = self.select_approval(
                &report_digest,
                base_revision,
                candidate_revision,
                candidate_tree,
                manifest_digest,
                rubric_digests,
                semantic_topology,
            )? {
                let created = OffsetDateTime::parse(&approval.created_at, &Rfc3339)
                    .context("approval created_at must be RFC3339")?;
                matches.push((
                    created,
                    approval_digest,
                    report_digest,
                    evaluation,
                    approval,
                ));
            }
        }
        matches.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        Ok(matches.into_iter().next().map(
            |(_, approval_digest, report_digest, evaluation, approval)| {
                (report_digest, evaluation, approval_digest, approval)
            },
        ))
    }

    pub fn write_attempt(&self, record: &PublicationAttemptRecord) -> Result<String> {
        record.validate()?;
        self.validate_attempt_references(record)?;
        let bytes = canonical_json(record)?;
        let digest = sha256_hex(&bytes);
        let directory = match record.update_kind {
            UpdateKind::Content => self
                .root()
                .join("attempts/content")
                .join(record.candidate_tree.as_deref().unwrap_or_default()),
            UpdateKind::DeletionOnly => self.root().join("attempts/deletions"),
            UpdateKind::Rejected => self.root().join("attempts/rejected"),
        };
        write_immutable(&directory.join(format!("{digest}.json")), &bytes, true)?;
        Ok(digest)
    }

    pub fn read_attempt(
        &self,
        update_kind: UpdateKind,
        candidate_tree: Option<&str>,
        digest: &str,
    ) -> Result<PublicationAttemptRecord> {
        let directory = match update_kind {
            UpdateKind::Content => {
                let tree = candidate_tree.context("content attempt requires candidate tree")?;
                validate_object_id(tree, "candidate_tree")?;
                self.root().join("attempts/content").join(tree)
            }
            UpdateKind::DeletionOnly => self.root().join("attempts/deletions"),
            UpdateKind::Rejected => self.root().join("attempts/rejected"),
        };
        let record: PublicationAttemptRecord =
            read_record(&record_path(&directory, digest)?, digest)?;
        record.validate()?;
        self.validate_attempt_references(&record)?;
        if record.update_kind != update_kind
            || (update_kind == UpdateKind::Content
                && record.candidate_tree.as_deref() != candidate_tree)
        {
            bail!("attempt storage path does not match record binding");
        }
        Ok(record)
    }

    fn validate_attempt_references(&self, record: &PublicationAttemptRecord) -> Result<()> {
        if record.update_kind != UpdateKind::Content {
            return Ok(());
        }
        let report_digest = record
            .evaluation_report_digest
            .as_deref()
            .unwrap_or_default();
        let evaluation = self.read_evaluation(report_digest)?;
        if evaluation.base_revision != record.base_revision.as_deref().unwrap_or_default()
            || evaluation.candidate_revision
                != record.candidate_revision.as_deref().unwrap_or_default()
            || evaluation.candidate_tree != record.candidate_tree.as_deref().unwrap_or_default()
            || evaluation.manifest_digest != record.manifest_digest.as_deref().unwrap_or_default()
            || Some(&evaluation.rubric_digests) != record.rubric_digests.as_ref()
            || evaluation.derived_disposition != record.derived_disposition
        {
            bail!("attempt binding or disposition does not match referenced evaluation");
        }
        if let Some(approval_digest) = &record.approval_digest {
            let approval = self.read_approval(report_digest, approval_digest)?;
            if !approval.matches_evaluation(report_digest, &evaluation) {
                bail!("attempt approval does not match referenced evaluation");
            }
        }
        Ok(())
    }

    fn report_path(&self, digest: &str) -> PathBuf {
        self.root().join("reports").join(format!("{digest}.json"))
    }

    fn report_path_checked(&self, digest: &str) -> Result<PathBuf> {
        validate_digest(digest, "report digest")?;
        Ok(self.report_path(digest))
    }

    fn approval_directory(&self, report_digest: &str) -> Result<PathBuf> {
        validate_digest(report_digest, "report digest")?;
        Ok(self.root().join("approvals").join(report_digest))
    }
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).context("failed to serialize canonical JSON")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn read_record<T: DeserializeOwned + Serialize>(path: &Path, digest: &str) -> Result<T> {
    validate_digest(digest, "record digest")?;
    let bytes =
        fs::read(path).with_context(|| format!("failed reading record {}", path.display()))?;
    let actual = sha256_hex(&bytes);
    if actual != digest {
        bail!("record digest mismatch: expected {digest}, found {actual}");
    }
    let record: T = serde_json::from_slice(&bytes)
        .with_context(|| format!("record is not valid closed JSON: {}", path.display()))?;
    let canonical = canonical_json(&record)?;
    if canonical != bytes {
        bail!("record is not canonical JSON: {}", path.display());
    }
    Ok(record)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteDisposition {
    Created,
    AlreadyExists,
}

fn write_immutable(path: &Path, bytes: &[u8], accept_identical: bool) -> Result<WriteDisposition> {
    let parent = path.parent().context("record path has no parent")?;
    create_durable_directory(parent)?;
    let temporary = parent.join(format!(".tmp-{}-{}", std::process::id(), Uuid::now_v7()));
    let mut temporary_created = false;
    let operation = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .with_context(|| format!("failed creating temporary record {}", temporary.display()))?;
        temporary_created = true;
        file.write_all(bytes)
            .context("failed writing temporary record")?;
        file.sync_all().context("failed syncing temporary record")?;
        match fs::hard_link(&temporary, path) {
            Ok(()) => {
                sync_directory(parent)?;
                Ok(WriteDisposition::Created)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if accept_identical {
                    let existing = fs::read(path).with_context(|| {
                        format!("failed reading existing record {}", path.display())
                    })?;
                    if existing != bytes {
                        bail!("immutable record collision at {}", path.display());
                    }
                }
                Ok(WriteDisposition::AlreadyExists)
            }
            Err(error) => Err(error)
                .with_context(|| format!("failed publishing immutable record {}", path.display())),
        }
    })();

    let cleanup = if temporary_created {
        fs::remove_file(&temporary)
            .with_context(|| format!("failed removing temporary record {}", temporary.display()))
            .and_then(|()| sync_directory(parent))
    } else {
        Ok(())
    };
    combine_operation_and_cleanup(operation, cleanup)
}

fn create_durable_directory(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => bail!(
            "record directory path is not a directory: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed inspecting record directory {}", path.display()));
        }
    }
    let parent = path.parent().context("record directory has no parent")?;
    create_durable_directory(parent)?;
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !fs::metadata(path)
                .with_context(|| format!("failed inspecting raced directory {}", path.display()))?
                .is_dir()
            {
                bail!(
                    "record directory path is not a directory: {}",
                    path.display()
                );
            }
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed creating record directory {}", path.display()));
        }
    }
    // Both creator and a raced AlreadyExists observer establish durability.
    // The observer cannot rely on the creator reaching either sync before crash.
    sync_directory(path)?;
    sync_directory(parent)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("failed syncing record directory {}", path.display()))
}

fn combine_operation_and_cleanup<T>(operation: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => {
            Err(error.context(format!("temporary record cleanup also failed: {cleanup:#}")))
        }
    }
}

fn record_path(directory: &Path, digest: &str) -> Result<PathBuf> {
    validate_digest(digest, "record digest")?;
    Ok(directory.join(format!("{digest}.json")))
}

fn require_same_binding(left: &CandidateBinding, right: &CandidateBinding) -> Result<()> {
    if left != right {
        bail!("deterministic and semantic candidate bindings differ");
    }
    Ok(())
}

fn require_record_binding(record: &EvaluationRecord, binding: &CandidateBinding) -> Result<()> {
    if record.base_revision != binding.base_revision
        || record.candidate_revision != binding.candidate_revision
        || record.candidate_tree != binding.candidate_tree
    {
        bail!("evaluation fields do not match deterministic candidate binding");
    }
    Ok(())
}

fn validate_digest(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("{label} must be a lowercase SHA-256 digest");
    }
    Ok(())
}

fn validate_object_id(value: &str, label: &str) -> Result<()> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("{label} must be a lowercase Git object ID");
    }
    Ok(())
}

fn validate_topology(
    topology: &SemanticTopology,
    rubric_digests: &BTreeMap<String, String>,
) -> Result<()> {
    if topology.axes.is_empty() {
        bail!("semantic topology must contain focused axes");
    }
    let mut ids = BTreeSet::new();
    let mut rubrics = BTreeSet::new();
    for binding in topology
        .axes
        .iter()
        .chain(std::iter::once(&topology.coherence))
    {
        if binding.id.is_empty() || binding.id.trim() != binding.id || !ids.insert(&binding.id) {
            bail!("semantic topology contains empty or duplicate result ID");
        }
        let rubric = binding
            .rubric
            .to_str()
            .context("semantic topology rubric path is not UTF-8")?;
        validate_repository_path(rubric, "semantic topology rubric")?;
        if !rubrics.insert(rubric.to_owned()) {
            bail!("semantic topology contains duplicate rubric path");
        }
    }
    let digest_paths = rubric_digests.keys().cloned().collect::<BTreeSet<_>>();
    if rubrics != digest_paths {
        bail!("semantic topology rubric paths do not exactly match rubric_digests");
    }
    Ok(())
}

fn validate_repository_path(value: &str, label: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("{label} must be a normalized repository-relative path");
    }
    Ok(())
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("failed formatting record timestamp")
}
