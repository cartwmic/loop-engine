use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;

use serde_json::Value;
use tempfile::TempDir;
use xtask::config::{SemanticRequirement, compute_binding, parse_manifest};
use xtask::process::EnvironmentChanges;
use xtask::quality::{CandidateBinding, DeterministicPhase, DeterministicResult};
use xtask::report::{
    DerivedDisposition, GateDecision, InputEvidence, InputKind, PublicationAttemptRecord,
    RejectionCode, SCHEMA_VERSION, Store, UpdateKind, UpdateTuple, canonical_json, sha256_hex,
};
use xtask::semantic_judge::{
    Citation, CitationKind, NormalizedResult, SemanticDisposition, SemanticResult, SemanticStatus,
};

const BASE: &str = "1111111111111111111111111111111111111111";
const CANDIDATE: &str = "2222222222222222222222222222222222222222";
const TREE: &str = "3333333333333333333333333333333333333333";
const SHA1_EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const SHA256_EMPTY_TREE: &str = "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321";

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {}: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn deterministic(pass: bool) -> DeterministicResult {
    DeterministicResult {
        phase: DeterministicPhase::Publication,
        binding: CandidateBinding {
            base_revision: BASE.to_owned(),
            candidate_revision: CANDIDATE.to_owned(),
            candidate_tree: TREE.to_owned(),
        },
        prerequisites: Vec::new(),
        checks: Vec::new(),
        final_source_verified: pass,
        final_failure: None,
    }
}

fn content_attempt_for_update(
    input_kind: InputKind,
    candidate: &str,
    remote: &str,
    base: &str,
) -> PublicationAttemptRecord {
    let tree = "3".repeat(candidate.len());
    let input = match input_kind {
        InputKind::GitUpdateLines => {
            format!("refs/heads/main {candidate} refs/heads/main {remote}\n")
        }
        InputKind::CiPushEvent => {
            format!(r#"{{"before":"{remote}","after":"{candidate}","ref":"refs/heads/main"}}"#)
        }
    };
    PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind: UpdateKind::Content,
        input_kind,
        input_evidence: InputEvidence {
            encoding: "utf-8".to_owned(),
            data: input,
        },
        updates: vec![UpdateTuple {
            local_ref: "refs/heads/main".to_owned(),
            local_sha: candidate.to_owned(),
            remote_ref: "refs/heads/main".to_owned(),
            remote_sha: remote.to_owned(),
        }],
        rejection_code: None,
        base_revision: Some(base.to_owned()),
        candidate_revision: Some(candidate.to_owned()),
        candidate_tree: Some(tree.clone()),
        manifest_digest: Some("4".repeat(64)),
        rubric_digests: Some(BTreeMap::from([(
            "quality/rubrics/test.md".to_owned(),
            "5".repeat(64),
        )])),
        fresh_deterministic_results: vec![DeterministicResult {
            phase: DeterministicPhase::Publication,
            binding: CandidateBinding {
                base_revision: base.to_owned(),
                candidate_revision: candidate.to_owned(),
                candidate_tree: tree,
            },
            prerequisites: Vec::new(),
            checks: Vec::new(),
            final_source_verified: true,
            final_failure: None,
        }],
        evaluation_report_digest: Some("6".repeat(64)),
        approval_digest: None,
        derived_disposition: DerivedDisposition::SemanticBlock,
        gate_decision: GateDecision::Block,
        created_at: "2026-07-25T00:00:00Z".to_owned(),
    }
}

fn normalized(id: &str, status: SemanticStatus) -> NormalizedResult {
    NormalizedResult {
        id: id.to_owned(),
        status,
        citations: vec![Citation {
            kind: CitationKind::Rubric,
            reference: format!("quality/rubrics/{id}.md"),
            detail: "fixture".to_owned(),
        }],
        message: "fixture result".to_owned(),
        attempts: Vec::new(),
        source_verified: Some(true),
    }
}

fn semantic(status: SemanticStatus) -> SemanticResult {
    let axes = [
        "documentation",
        "observability",
        "architecture",
        "behavioral-evidence",
    ]
    .into_iter()
    .map(|id| normalized(id, status))
    .collect();
    let disposition = if status == SemanticStatus::Pass {
        SemanticDisposition::Pass
    } else {
        SemanticDisposition::SemanticBlock
    };
    SemanticResult {
        binding: deterministic(true).binding,
        axes,
        coherence: normalized("coherence", status),
        disposition,
        source_mutation: None,
    }
}

fn binding(root: &Path) -> xtask::config::BindingDigests {
    let manifest = br#"schema_version = 2
[defaults]
timeout_seconds = 1
max_output_bytes = 1
[runner]
inputs = ["quality/manifest.toml"]
[[checks]]
id = "publication"
phases = ["publication"]
scope = "repository"
program = "/usr/bin/true"
args = []
cwd = "{candidate_root}"
[semantic]
program = "/usr/bin/true"
args = []
cwd = "{candidate_root}"
response_schema = "quality/semantic-judge/v2/response.schema.json"
[[semantic.axes]]
id = "documentation"
rubric = "quality/rubrics/documentation.md"
[[semantic.axes]]
id = "observability"
rubric = "quality/rubrics/observability.md"
[[semantic.axes]]
id = "architecture"
rubric = "quality/rubrics/architecture.md"
[[semantic.axes]]
id = "behavioral-evidence"
rubric = "quality/rubrics/behavioral-evidence.md"
[semantic.coherence]
id = "coherence"
rubric = "quality/rubrics/coherence.md"
"#;
    for id in [
        "documentation",
        "observability",
        "architecture",
        "behavioral-evidence",
        "coherence",
    ] {
        let path = root.join(format!("quality/rubrics/{id}.md"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("# {id}\n")).unwrap();
    }
    let document = parse_manifest(manifest, SemanticRequirement::Required).unwrap();
    compute_binding(&document, root).unwrap()
}

fn evaluation(
    root: &Path,
    deterministic_pass: bool,
    semantic_status: Option<SemanticStatus>,
) -> xtask::report::EvaluationRecord {
    xtask::report::EvaluationRecord::new(
        deterministic(deterministic_pass),
        semantic_status.map(semantic),
        &binding(root),
    )
    .unwrap()
}

#[test]
fn canonical_json_and_sha256_are_exact_and_stable() {
    let value: BTreeMap<&str, u8> = [("b", 1), ("a", 2)].into_iter().collect();
    let bytes = canonical_json(&value).unwrap();
    assert_eq!(bytes, br#"{"a":2,"b":1}"#);
    assert_eq!(
        sha256_hex(&bytes),
        "d3626ac30a87e6f7a6428233b3c68299976865fa5508e4267c5415c76af7a772"
    );
    assert!(std::str::from_utf8(&bytes).is_ok());
    assert!(!bytes.ends_with(b"\n"));
}

#[test]
fn evaluation_roundtrip_checks_digest_and_canonical_bytes() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let record = evaluation(root.path(), true, Some(SemanticStatus::Block));
    let digest = store.write_evaluation(&record).unwrap();
    let path = store.root().join("reports").join(format!("{digest}.json"));
    let bytes = fs::read(&path).unwrap();
    assert_eq!(sha256_hex(&bytes), digest);
    let read = store.read_evaluation(&digest).unwrap();
    assert_eq!(read.derived_disposition, DerivedDisposition::SemanticBlock);
    assert_eq!(read.axis_results.len(), 4);

    fs::write(&path, b"{}" as &[u8]).unwrap();
    assert!(
        store
            .read_evaluation(&digest)
            .unwrap_err()
            .to_string()
            .contains("digest mismatch")
    );
    assert!(store.write_evaluation(&record).is_err());
    assert_eq!(fs::read(path).unwrap(), b"{}");
}

#[test]
fn concurrent_first_writers_leave_durable_canonical_record_without_temps() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let record = evaluation(root.path(), true, Some(SemanticStatus::Block));

    let writes = (0..16)
        .map(|_| {
            let store = store.clone();
            let record = record.clone();
            thread::spawn(move || store.write_evaluation(&record).unwrap())
        })
        .collect::<Vec<_>>();
    let digests = writes
        .into_iter()
        .map(|write| write.join().unwrap())
        .collect::<Vec<_>>();

    assert!(digests.windows(2).all(|pair| pair[0] == pair[1]));
    let stored = store.read_evaluation(&digests[0]).unwrap();
    assert_eq!(
        canonical_json(&stored).unwrap(),
        canonical_json(&record).unwrap()
    );
    for entry in fs::read_dir(store.root().join("reports")).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tmp-")
        );
    }
}

#[test]
fn crash_residue_in_unscanned_temp_directories_does_not_block_approval_lookup() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let record = evaluation(root.path(), true, Some(SemanticStatus::Block));
    let report_digest = store.write_evaluation(&record).unwrap();
    let _ = store.approve(&report_digest, "crash residue test").unwrap();

    for directory in [
        store.root().join(".tmp"),
        store.root().join("approvals/.tmp"),
    ] {
        assert!(directory.is_dir(), "missing {}", directory.display());
        fs::write(directory.join(".tmp-crash-residue"), b"incomplete").unwrap();
    }

    let selected = store
        .select_approved_evaluation(
            &record.base_revision,
            &record.candidate_revision,
            &record.candidate_tree,
            &record.manifest_digest,
            &record.rubric_digests,
            &record.semantic_topology,
        )
        .unwrap()
        .expect("approval remains discoverable");
    assert_eq!(selected.0, report_digest);
}

#[test]
fn noncanonical_and_unknown_field_records_are_rejected() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let record = evaluation(root.path(), false, None);
    let mut value = serde_json::to_value(&record).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unknown".to_owned(), Value::Bool(true));
    let bytes = serde_json::to_vec(&value).unwrap();
    let digest = sha256_hex(&bytes);
    let path = store.root().join("reports").join(format!("{digest}.json"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    assert!(
        store
            .read_evaluation(&digest)
            .unwrap_err()
            .to_string()
            .contains("closed JSON")
    );

    let canonical = canonical_json(&record).unwrap();
    let mut spaced = canonical.clone();
    spaced.push(b'\n');
    let digest = sha256_hex(&spaced);
    let path = store.root().join("reports").join(format!("{digest}.json"));
    fs::write(path, spaced).unwrap();
    assert!(
        store
            .read_evaluation(&digest)
            .unwrap_err()
            .to_string()
            .contains("not canonical")
    );
}

#[test]
fn evaluation_nullability_and_binding_are_fail_closed() {
    let root = TempDir::new().unwrap();
    let mut blocked = evaluation(root.path(), false, None);
    blocked
        .axis_results
        .push(normalized("documentation", SemanticStatus::Block));
    assert!(
        blocked
            .validate()
            .unwrap_err()
            .to_string()
            .contains("nullability")
    );

    let mut semantic_block = evaluation(root.path(), true, Some(SemanticStatus::Block));
    semantic_block.candidate_tree = "4444444444444444444444444444444444444444".to_owned();
    assert!(
        semantic_block
            .validate()
            .unwrap_err()
            .to_string()
            .contains("binding")
    );

    assert!(
        xtask::report::EvaluationRecord::new(
            deterministic(false),
            Some(semantic(SemanticStatus::Block)),
            &binding(root.path())
        )
        .is_err()
    );
}

#[test]
fn semantic_topology_requires_exact_ordered_results_and_rubric_bindings() {
    let root = TempDir::new().unwrap();
    let record = evaluation(root.path(), true, Some(SemanticStatus::Block));

    let mut reordered_results = record.clone();
    reordered_results.axis_results.swap(0, 1);
    assert!(
        reordered_results
            .validate()
            .unwrap_err()
            .to_string()
            .contains("ordered topology")
    );

    let mut changed_coherence = record.clone();
    changed_coherence.coherence_result.as_mut().unwrap().id = "other".to_owned();
    assert!(
        changed_coherence
            .validate()
            .unwrap_err()
            .to_string()
            .contains("coherence result ID")
    );

    let mut changed_rubric = record;
    changed_rubric.semantic_topology.axes[0].rubric = "quality/rubrics/other.md".into();
    assert!(
        changed_rubric
            .validate()
            .unwrap_err()
            .to_string()
            .contains("rubric paths")
    );
}

#[test]
fn approval_accepts_only_verified_semantic_block_and_repeats_distinctly() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let report_digest = store
        .write_evaluation(&evaluation(root.path(), true, Some(SemanticStatus::Block)))
        .unwrap();
    assert!(store.approve(&report_digest, "  ").is_err());

    let (first_digest, first) = store.approve(&report_digest, "owner accepts risk").unwrap();
    let (second_digest, second) = store.approve(&report_digest, "owner accepts risk").unwrap();
    assert_ne!(first.approval_id, second.approval_id);
    assert_ne!(first_digest, second_digest);
    assert_eq!(
        uuid::Uuid::parse_str(&first.approval_id)
            .unwrap()
            .get_version_num(),
        7
    );
    assert_eq!(
        store
            .read_approval(&report_digest, &first_digest)
            .unwrap()
            .reason,
        "owner accepts risk"
    );

    let deterministic_digest = store
        .write_evaluation(&evaluation(root.path(), false, None))
        .unwrap();
    assert!(
        store
            .approve(&deterministic_digest, "cannot bypass")
            .is_err()
    );
    let pass_digest = store
        .write_evaluation(&evaluation(root.path(), true, Some(SemanticStatus::Pass)))
        .unwrap();
    assert!(store.approve(&pass_digest, "not a failure").is_err());
}

#[test]
fn approval_binding_predicate_invalidates_every_bound_component() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let report = evaluation(root.path(), true, Some(SemanticStatus::Block));
    let report_digest = store.write_evaluation(&report).unwrap();
    let (_, approval) = store.approve(&report_digest, "accepted").unwrap();
    assert!(approval.matches_binding(
        &report.base_revision,
        &report.candidate_revision,
        &report.candidate_tree,
        &report.manifest_digest,
        &report.rubric_digests,
        &report.semantic_topology,
    ));
    let changed = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(!approval.matches_binding(
        changed,
        &report.candidate_revision,
        &report.candidate_tree,
        &report.manifest_digest,
        &report.rubric_digests,
        &report.semantic_topology,
    ));
    assert!(!approval.matches_binding(
        &report.base_revision,
        changed,
        &report.candidate_tree,
        &report.manifest_digest,
        &report.rubric_digests,
        &report.semantic_topology,
    ));
    assert!(!approval.matches_binding(
        &report.base_revision,
        &report.candidate_revision,
        changed,
        &report.manifest_digest,
        &report.rubric_digests,
        &report.semantic_topology,
    ));
    assert!(!approval.matches_binding(
        &report.base_revision,
        &report.candidate_revision,
        &report.candidate_tree,
        &"a".repeat(64),
        &report.rubric_digests,
        &report.semantic_topology,
    ));
    let mut rubrics = report.rubric_digests.clone();
    *rubrics.values_mut().next().unwrap() = "b".repeat(64);
    assert!(!approval.matches_binding(
        &report.base_revision,
        &report.candidate_revision,
        &report.candidate_tree,
        &report.manifest_digest,
        &rubrics,
        &report.semantic_topology,
    ));
    let mut topology = report.semantic_topology.clone();
    topology.axes.swap(0, 1);
    assert!(!approval.matches_binding(
        &report.base_revision,
        &report.candidate_revision,
        &report.candidate_tree,
        &report.manifest_digest,
        &report.rubric_digests,
        &topology,
    ));
}

#[test]
fn approval_corruption_and_report_binding_mismatch_are_rejected() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let report_digest = store
        .write_evaluation(&evaluation(root.path(), true, Some(SemanticStatus::Block)))
        .unwrap();
    let (approval_digest, approval) = store.approve(&report_digest, "accepted").unwrap();
    let directory = store.root().join("approvals").join(&report_digest);
    let mut tampered_topology = approval.clone();
    tampered_topology.semantic_topology.axes.swap(0, 1);
    let tampered_bytes = canonical_json(&tampered_topology).unwrap();
    let tampered_digest = sha256_hex(&tampered_bytes);
    fs::write(
        directory.join(format!("{tampered_digest}.json")),
        tampered_bytes,
    )
    .unwrap();
    assert!(
        store
            .read_approval(&report_digest, &tampered_digest)
            .unwrap_err()
            .to_string()
            .contains("binding")
    );

    let path = directory.join(format!("{approval_digest}.json"));
    fs::write(path, b"{}" as &[u8]).unwrap();
    assert!(
        store
            .read_approval(&report_digest, &approval_digest)
            .is_err()
    );
}

#[test]
fn attempt_nullability_roundtrip_and_path_binding_are_checked() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let deletion = PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind: UpdateKind::DeletionOnly,
        input_kind: InputKind::GitUpdateLines,
        input_evidence: InputEvidence {
            encoding: "utf-8".to_owned(),
            data: String::new(),
        },
        updates: Vec::new(),
        rejection_code: None,
        base_revision: None,
        candidate_revision: None,
        candidate_tree: None,
        manifest_digest: None,
        rubric_digests: None,
        fresh_deterministic_results: Vec::new(),
        evaluation_report_digest: None,
        approval_digest: None,
        derived_disposition: DerivedDisposition::Pass,
        gate_decision: GateDecision::Pass,
        created_at: "2026-07-25T00:00:00Z".to_owned(),
    };
    let digest = store.write_attempt(&deletion).unwrap();
    assert_eq!(
        store
            .read_attempt(UpdateKind::DeletionOnly, None, &digest)
            .unwrap()
            .update_kind,
        UpdateKind::DeletionOnly
    );

    let mut rejected = deletion.clone();
    rejected.update_kind = UpdateKind::Rejected;
    rejected.input_evidence.data = "bad\n".to_owned();
    rejected.rejection_code = Some(RejectionCode::MalformedUpdateInput);
    rejected.derived_disposition = DerivedDisposition::DeterministicBlock;
    rejected.gate_decision = GateDecision::Block;
    assert!(rejected.validate().is_ok());
    rejected.candidate_tree = Some(TREE.to_owned());
    assert!(rejected.validate().is_err());
}

#[test]
fn content_attempt_store_checks_report_approval_and_policy_bindings() {
    let root = TempDir::new().unwrap();
    let store = Store::from_common_directory(root.path());
    let report = evaluation(root.path(), true, Some(SemanticStatus::Block));
    let report_digest = store.write_evaluation(&report).unwrap();
    let (approval_digest, _) = store.approve(&report_digest, "accepted").unwrap();
    let attempt = PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind: UpdateKind::Content,
        input_kind: InputKind::GitUpdateLines,
        input_evidence: InputEvidence {
            encoding: "utf-8".to_owned(),
            data: format!("refs/heads/main {CANDIDATE} refs/heads/main {BASE}\n"),
        },
        updates: vec![UpdateTuple {
            local_ref: "refs/heads/main".to_owned(),
            local_sha: CANDIDATE.to_owned(),
            remote_ref: "refs/heads/main".to_owned(),
            remote_sha: BASE.to_owned(),
        }],
        rejection_code: None,
        base_revision: Some(report.base_revision.clone()),
        candidate_revision: Some(report.candidate_revision.clone()),
        candidate_tree: Some(report.candidate_tree.clone()),
        manifest_digest: Some(report.manifest_digest.clone()),
        rubric_digests: Some(report.rubric_digests.clone()),
        fresh_deterministic_results: vec![deterministic(true)],
        evaluation_report_digest: Some(report_digest.clone()),
        approval_digest: Some(approval_digest),
        derived_disposition: DerivedDisposition::SemanticBlock,
        gate_decision: GateDecision::Approved,
        created_at: "2026-07-25T00:00:00Z".to_owned(),
    };
    let digest = store.write_attempt(&attempt).unwrap();
    assert_eq!(
        store
            .read_attempt(UpdateKind::Content, Some(TREE), &digest)
            .unwrap()
            .gate_decision,
        GateDecision::Approved
    );

    let mut changed_manifest = attempt;
    changed_manifest.manifest_digest = Some("a".repeat(64));
    assert!(store.write_attempt(&changed_manifest).is_err());
}

#[test]
fn attempt_validation_rederives_input_and_deterministic_disposition() {
    let deletion_input = format!("(delete) {} refs/heads/old {BASE}\n", "0".repeat(40));
    let mut deletion = PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind: UpdateKind::DeletionOnly,
        input_kind: InputKind::GitUpdateLines,
        input_evidence: InputEvidence {
            encoding: "utf-8".to_owned(),
            data: deletion_input,
        },
        updates: vec![UpdateTuple {
            local_ref: "(delete)".to_owned(),
            local_sha: "0".repeat(40),
            remote_ref: "refs/heads/old".to_owned(),
            remote_sha: BASE.to_owned(),
        }],
        rejection_code: None,
        base_revision: None,
        candidate_revision: None,
        candidate_tree: None,
        manifest_digest: None,
        rubric_digests: None,
        fresh_deterministic_results: Vec::new(),
        evaluation_report_digest: None,
        approval_digest: None,
        derived_disposition: DerivedDisposition::Pass,
        gate_decision: GateDecision::Pass,
        created_at: "2026-07-25T00:00:00Z".to_owned(),
    };
    assert!(deletion.validate().is_ok());

    let mut ci_deletion = deletion.clone();
    ci_deletion.input_kind = InputKind::CiPushEvent;
    ci_deletion.input_evidence.data = format!(
        r#"{{"before":"{BASE}","after":"{}","ref":"refs/heads/old"}}"#,
        "0".repeat(40)
    );
    ci_deletion.updates = vec![UpdateTuple {
        local_ref: "refs/heads/old".to_owned(),
        local_sha: "0".repeat(40),
        remote_ref: "refs/heads/old".to_owned(),
        remote_sha: BASE.to_owned(),
    }];
    assert!(ci_deletion.validate().is_ok());
    ci_deletion.input_evidence.data.push('\n');
    assert!(ci_deletion.validate().is_err());

    deletion.input_evidence.data = format!("refs/heads/main {CANDIDATE} refs/heads/main {BASE}\n");
    deletion.updates = vec![UpdateTuple {
        local_ref: "refs/heads/main".to_owned(),
        local_sha: CANDIDATE.to_owned(),
        remote_ref: "refs/heads/main".to_owned(),
        remote_sha: BASE.to_owned(),
    }];
    assert!(
        deletion
            .validate()
            .unwrap_err()
            .to_string()
            .contains("classification")
    );

    deletion.input_evidence = InputEvidence {
        encoding: "base64".to_owned(),
        data: "AA=A".to_owned(),
    };
    assert!(format!("{:#}", deletion.validate().unwrap_err()).contains("base64"));

    let root = TempDir::new().unwrap();
    let report = evaluation(root.path(), true, Some(SemanticStatus::Block));
    let mut content = PublicationAttemptRecord {
        schema_version: SCHEMA_VERSION,
        update_kind: UpdateKind::Content,
        input_kind: InputKind::GitUpdateLines,
        input_evidence: InputEvidence {
            encoding: "utf-8".to_owned(),
            data: format!("refs/heads/main {CANDIDATE} refs/heads/main {BASE}\n"),
        },
        updates: vec![UpdateTuple {
            local_ref: "refs/heads/main".to_owned(),
            local_sha: CANDIDATE.to_owned(),
            remote_ref: "refs/heads/main".to_owned(),
            remote_sha: BASE.to_owned(),
        }],
        rejection_code: None,
        base_revision: Some(BASE.to_owned()),
        candidate_revision: Some(CANDIDATE.to_owned()),
        candidate_tree: Some(TREE.to_owned()),
        manifest_digest: Some(report.manifest_digest),
        rubric_digests: Some(report.rubric_digests),
        fresh_deterministic_results: vec![deterministic(true)],
        evaluation_report_digest: Some("a".repeat(64)),
        approval_digest: None,
        derived_disposition: DerivedDisposition::SemanticBlock,
        gate_decision: GateDecision::Block,
        created_at: "2026-07-25T00:00:00Z".to_owned(),
    };
    assert!(content.validate().is_ok());
    let mut forged_ci_approval = content.clone();
    forged_ci_approval.input_kind = InputKind::CiPushEvent;
    forged_ci_approval.input_evidence.data =
        format!(r#"{{"before":"{BASE}","after":"{CANDIDATE}","ref":"refs/heads/main"}}"#);
    forged_ci_approval.approval_digest = Some("b".repeat(64));
    forged_ci_approval.gate_decision = GateDecision::Approved;
    assert!(
        forged_ci_approval
            .validate()
            .unwrap_err()
            .to_string()
            .contains("cannot reference local approval")
    );
    content.fresh_deterministic_results = vec![deterministic(false)];
    assert!(
        content
            .validate()
            .unwrap_err()
            .to_string()
            .contains("contradicts")
    );
    content.fresh_deterministic_results = vec![deterministic(true)];
    content.derived_disposition = DerivedDisposition::DeterministicBlock;
    content.gate_decision = GateDecision::Block;
    assert!(
        content
            .validate()
            .unwrap_err()
            .to_string()
            .contains("contradicts")
    );
}

#[test]
fn content_attempt_revisions_bind_exact_parsed_content_update() {
    for input_kind in [InputKind::GitUpdateLines, InputKind::CiPushEvent] {
        let ordinary = content_attempt_for_update(input_kind, CANDIDATE, BASE, BASE);
        assert!(ordinary.validate().is_ok());

        let mut forged_candidate = ordinary.clone();
        let forged_oid = "7".repeat(40);
        forged_candidate.candidate_revision = Some(forged_oid.clone());
        forged_candidate.fresh_deterministic_results[0]
            .binding
            .candidate_revision = forged_oid;
        assert!(
            forged_candidate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("candidate_revision")
        );

        let mut forged_base = ordinary;
        let forged_oid = "8".repeat(40);
        forged_base.base_revision = Some(forged_oid.clone());
        forged_base.fresh_deterministic_results[0]
            .binding
            .base_revision = forged_oid;
        assert!(
            forged_base
                .validate()
                .unwrap_err()
                .to_string()
                .contains("base_revision")
        );

        for (width, empty_tree) in [(40, SHA1_EMPTY_TREE), (64, SHA256_EMPTY_TREE)] {
            let candidate = "2".repeat(width);
            let zero = "0".repeat(width);
            let valid_new_branch =
                content_attempt_for_update(input_kind, &candidate, &zero, empty_tree);
            assert!(valid_new_branch.validate().is_ok());

            let mut wrong_empty_tree = valid_new_branch;
            let wrong_oid = "9".repeat(width);
            wrong_empty_tree.base_revision = Some(wrong_oid.clone());
            wrong_empty_tree.fresh_deterministic_results[0]
                .binding
                .base_revision = wrong_oid;
            assert!(
                wrong_empty_tree
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("base_revision")
            );
        }
    }

    let deletion_remote = "9".repeat(40);
    let zero = "0".repeat(40);
    let mut mixed = content_attempt_for_update(InputKind::GitUpdateLines, CANDIDATE, BASE, BASE);
    mixed.input_evidence.data = format!(
        "(delete) {zero} refs/heads/old {deletion_remote}\nrefs/heads/main {CANDIDATE} refs/heads/main {BASE}\n"
    );
    mixed.updates.insert(
        0,
        UpdateTuple {
            local_ref: "(delete)".to_owned(),
            local_sha: zero,
            remote_ref: "refs/heads/old".to_owned(),
            remote_sha: deletion_remote,
        },
    );
    assert!(mixed.validate().is_ok());
}

#[test]
fn durable_store_creates_full_hierarchy_and_leaves_no_temporary_records() {
    let root = TempDir::new().unwrap();
    let common = root.path().join("nested/git/common");
    fs::create_dir_all(&common).unwrap();
    let store = Store::from_common_directory(&common);
    let report_digest = store
        .write_evaluation(&evaluation(root.path(), true, Some(SemanticStatus::Block)))
        .unwrap();
    let _ = store.approve(&report_digest, "durable hierarchy").unwrap();

    for directory in [
        store.root(),
        store.root().join("reports"),
        store.root().join("approvals").join(report_digest),
    ] {
        assert!(directory.is_dir(), "missing {}", directory.display());
        assert!(fs::read_dir(directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tmp-")
        }));
    }
}

#[test]
fn tracked_record_schemas_are_closed_and_parseable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for name in ["evaluation", "approval", "attempt"] {
        let bytes =
            fs::read(root.join(format!("quality/publication-report/v1/{name}.schema.json")))
                .unwrap();
        let schema: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(schema["additionalProperties"], false, "{name}");
        assert!(schema["required"].as_array().unwrap().len() >= 10);
    }
    let definitions: Value = serde_json::from_slice(
        &fs::read(root.join("quality/publication-report/v1/definitions.schema.json")).unwrap(),
    )
    .unwrap();
    assert!(definitions["$defs"].as_object().unwrap().len() >= 10);
}

#[test]
fn linked_worktree_store_uses_authoritative_git_common_directory() {
    let primary = TempDir::new().unwrap();
    git(primary.path(), &["init", "-b", "main"]);
    git(primary.path(), &["config", "user.email", "report@test"]);
    git(primary.path(), &["config", "user.name", "Report Test"]);
    fs::write(primary.path().join("tracked"), "content\n").unwrap();
    git(primary.path(), &["add", "-A"]);
    git(primary.path(), &["commit", "-m", "base"]);
    let linked_parent = TempDir::new().unwrap();
    let linked = linked_parent.path().join("linked");
    git(
        primary.path(),
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );

    let store = Store::open(&linked).unwrap();
    let expected_common = primary.path().join(".git").canonicalize().unwrap();
    assert!(store.root().starts_with(&expected_common));
    assert!(!store.root().starts_with(linked.join(".git")));
}

#[test]
fn deserialized_environment_shape_remains_closed() {
    let value = serde_json::json!({"set": {}, "unset": [], "extra": true});
    assert!(serde_json::from_value::<EnvironmentChanges>(value).is_err());
}
