//! Aggregate one-tip publication validation and immutable attempt evidence.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context, Result, bail};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::candidate::{Candidate, PreparedCandidate};
use crate::config::{self, BindingDigests, Phase, SemanticRequirement};
use crate::process::Cancellation;
use crate::publication_input::UpdateTuple;
pub use crate::publication_input::{ParsedUpdateDisposition, ParsedUpdates, parse_updates};
use crate::report::{
    DerivedDisposition, EvaluationRecord, GateDecision, InputKind, PublicationAttemptRecord,
    RejectionCode, SCHEMA_VERSION, Store, UpdateKind,
};

#[derive(Debug)]
pub struct PublicationOutcome {
    pub attempt_digest: String,
    pub attempt: PublicationAttemptRecord,
}

/// Validate one aggregate Git update input and write exactly one attempt when
/// input classification or candidate execution reaches a reportable verdict.
pub fn run_publication(
    repository_path: &Path,
    input: &[u8],
    cancellation: &Cancellation,
) -> Result<PublicationOutcome> {
    let parsed = parse_updates(input);
    match parsed.disposition.clone() {
        ParsedUpdateDisposition::Rejected(code) => {
            let store = narrow_store(repository_path)?;
            finish_without_children(cancellation)?;
            let attempt = non_content_attempt(&parsed, UpdateKind::Rejected, Some(code))?;
            write_outcome(&store, attempt)
        }
        ParsedUpdateDisposition::DeletionOnly => {
            let store = narrow_store(repository_path)?;
            finish_without_children(cancellation)?;
            let attempt = non_content_attempt(&parsed, UpdateKind::DeletionOnly, None)?;
            write_outcome(&store, attempt)
        }
        ParsedUpdateDisposition::Content(update) => {
            run_content(repository_path, parsed, update, cancellation)
        }
    }
}

fn run_content(
    repository_path: &Path,
    parsed: ParsedUpdates,
    update: UpdateTuple,
    cancellation: &Cancellation,
) -> Result<PublicationOutcome> {
    let base = if is_zero_oid(&update.remote_sha) {
        None
    } else {
        Some(OsStr::new(&update.remote_sha))
    };
    let candidate = Candidate::revision(repository_path, base, OsStr::new(&update.local_sha))
        .context("failed to materialize publication candidate")?
        .prepare(SemanticRequirement::Required)
        .context("failed to prepare publication candidate")?;
    let pending = build_content_pending(&candidate, parsed, cancellation);
    let cleanup = candidate
        .cleanup()
        .map_err(anyhow::Error::new)
        .context("failed to clean publication candidate state");
    let pending = combine_operation_and_cleanup(pending, cleanup)?;
    if cancellation.is_cancelled() || !cancellation.finish() {
        bail!("publication validation interrupted before attempt storage");
    }

    let report_digest = match pending.evaluation {
        Some(evaluation) => pending.store.write_evaluation(&evaluation)?,
        None => pending
            .existing_report_digest
            .context("approved publication is missing evaluation report digest")?,
    };
    let mut attempt = pending.attempt;
    attempt.evaluation_report_digest = Some(report_digest);
    write_outcome(&pending.store, attempt)
}

struct PendingContent {
    store: Store,
    evaluation: Option<EvaluationRecord>,
    existing_report_digest: Option<String>,
    attempt: PublicationAttemptRecord,
}

fn build_content_pending(
    candidate: &PreparedCandidate,
    parsed: ParsedUpdates,
    cancellation: &Cancellation,
) -> Result<PendingContent> {
    // Capture every policy binding before configured children can mutate source.
    let binding = config::compute_binding(candidate.manifest(), candidate.source_root())
        .context("failed to compute publication policy binding")?;
    let rubric_digests = report_rubric_digests(&binding)?;
    let topology = binding
        .semantic_topology()
        .context("publication binding requires semantic topology")?
        .clone();
    let store = Store::from_repository(candidate.repository());

    let deterministic =
        crate::quality::run_with_cancellation(candidate, Phase::Publication, cancellation);
    if cancellation.is_cancelled() {
        bail!("publication validation interrupted during deterministic execution");
    }

    let (evaluation, existing_report_digest, approval_digest, disposition, gate_decision) =
        if !deterministic.passed() {
            (
                Some(EvaluationRecord::new(
                    deterministic.clone(),
                    None,
                    &binding,
                )?),
                None,
                None,
                DerivedDisposition::DeterministicBlock,
                GateDecision::Block,
            )
        } else if let Some((report_digest, evaluation, approval_digest, approval)) = store
            .select_approved_evaluation(
                candidate.base_revision(),
                candidate.candidate_revision(),
                candidate.candidate_tree(),
                binding.manifest_digest(),
                &rubric_digests,
                &topology,
            )?
        {
            if !approval.matches_evaluation(&report_digest, &evaluation)
                || !approval.matches_binding(
                    candidate.base_revision(),
                    candidate.candidate_revision(),
                    candidate.candidate_tree(),
                    binding.manifest_digest(),
                    &rubric_digests,
                    &topology,
                )
            {
                bail!("selected approval failed exact publication binding predicate");
            }
            (
                None,
                Some(report_digest),
                Some(approval_digest),
                evaluation.derived_disposition,
                GateDecision::Approved,
            )
        } else {
            let semantic = crate::semantic_judge::run_with_cancellation(
                candidate,
                &deterministic,
                cancellation,
            )
            .context("failed to run publication semantic validation")?;
            if cancellation.is_cancelled() {
                bail!("publication validation interrupted during semantic execution");
            }
            let evaluation =
                EvaluationRecord::new(deterministic.clone(), Some(semantic), &binding)?;
            let disposition = evaluation.derived_disposition;
            let gate = if disposition == DerivedDisposition::Pass {
                GateDecision::Pass
            } else {
                GateDecision::Block
            };
            (Some(evaluation), None, None, disposition, gate)
        };

    let attempt = PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind: UpdateKind::Content,
        input_kind: InputKind::GitUpdateLines,
        input_evidence: parsed.input_evidence,
        updates: parsed.updates,
        rejection_code: None,
        base_revision: Some(candidate.base_revision().to_owned()),
        candidate_revision: Some(candidate.candidate_revision().to_owned()),
        candidate_tree: Some(candidate.candidate_tree().to_owned()),
        manifest_digest: Some(binding.manifest_digest().to_owned()),
        rubric_digests: Some(rubric_digests),
        fresh_deterministic_results: vec![deterministic],
        evaluation_report_digest: None,
        approval_digest,
        derived_disposition: disposition,
        gate_decision,
        created_at: now_rfc3339()?,
    };
    Ok(PendingContent {
        store,
        evaluation,
        existing_report_digest,
        attempt,
    })
}

fn non_content_attempt(
    parsed: &ParsedUpdates,
    update_kind: UpdateKind,
    rejection_code: Option<RejectionCode>,
) -> Result<PublicationAttemptRecord> {
    let rejected = update_kind == UpdateKind::Rejected;
    Ok(PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind,
        input_kind: InputKind::GitUpdateLines,
        input_evidence: parsed.input_evidence.clone(),
        updates: parsed.updates.clone(),
        rejection_code,
        base_revision: None,
        candidate_revision: None,
        candidate_tree: None,
        manifest_digest: None,
        rubric_digests: None,
        fresh_deterministic_results: Vec::new(),
        evaluation_report_digest: None,
        approval_digest: None,
        derived_disposition: if rejected {
            DerivedDisposition::DeterministicBlock
        } else {
            DerivedDisposition::Pass
        },
        gate_decision: if rejected {
            GateDecision::Block
        } else {
            GateDecision::Pass
        },
        created_at: now_rfc3339()?,
    })
}

fn is_zero_oid(value: &str) -> bool {
    value.bytes().all(|byte| byte == b'0')
}

fn narrow_store(repository_path: &Path) -> Result<Store> {
    Ok(Store::from_common_directory(
        &crate::git::common_directory_only(repository_path)?,
    ))
}

fn finish_without_children(cancellation: &Cancellation) -> Result<()> {
    if cancellation.is_cancelled() || !cancellation.finish() {
        bail!("publication validation interrupted before attempt storage");
    }
    Ok(())
}

fn write_outcome(store: &Store, attempt: PublicationAttemptRecord) -> Result<PublicationOutcome> {
    let attempt_digest = store.write_attempt(&attempt)?;
    Ok(PublicationOutcome {
        attempt_digest,
        attempt,
    })
}

fn report_rubric_digests(binding: &BindingDigests) -> Result<BTreeMap<String, String>> {
    binding
        .rubric_digests()
        .iter()
        .map(|(path, digest)| {
            path.to_str()
                .map(|path| (path.to_owned(), digest.clone()))
                .ok_or_else(|| anyhow::anyhow!("rubric path is not UTF-8: {}", path.display()))
        })
        .collect()
}

fn now_rfc3339() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .context("failed formatting publication timestamp")
}

fn combine_operation_and_cleanup<T>(operation: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(error.context(format!(
            "publication candidate cleanup also failed: {cleanup:#}"
        ))),
    }
}
